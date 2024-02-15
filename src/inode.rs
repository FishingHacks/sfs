use std::mem::{size_of, MaybeUninit};

use crate::{
    directory::DirEntry,
    disk::DiskError,
    fs::{FileSystem, FsError, BLOCK_SIZE, INODES_PER_BLOCK},
};

#[derive(Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum InodeType {
    FiFo = 0x1000,
    CharacterDevice = 0x2000,
    Directory = 0x4000,
    BlockDevice = 0x6000,
    File = 0x8000,
    Socket = 0xa000,
    Unknown(u16),
}

impl InodeType {
    pub fn as_u16(&self) -> u16 {
        match self {
            Self::FiFo => 0x1000,
            Self::CharacterDevice => 0x2000,
            Self::Directory => 0x4000,
            Self::BlockDevice => 0x6000,
            Self::File => 0x8000,
            Self::Socket => 0xa000,
            Self::Unknown(other) => *other,
        }
    }
}

#[repr(u16)]
pub enum Permission {
    OtherExecute = 0o0001,
    OtherWrite = 0o0002,
    OtherRead = 0o0004,

    GroupExecute = 0o0010,
    GroupWrite = 0o0020,
    GroupRead = 0o0040,

    UserExecute = 0o0100,
    UserWrite = 0o0200,
    UserRead = 0o0400,

    Sticky = 0o1000,
    SetGid = 0o2000,
    SetUid = 0o4000,
    Other(u16),
}

impl Permission {
    fn as_u16(&self) -> u16 {
        match self {
            Self::OtherExecute => 0o0001,
            Self::OtherWrite => 0o0002,
            Self::OtherRead => 0o0004,
            Self::GroupExecute => 0o0010,
            Self::GroupWrite => 0o0020,
            Self::GroupRead => 0o0040,
            Self::UserExecute => 0o0100,
            Self::UserWrite => 0o0200,
            Self::UserRead => 0o0400,
            Self::Sticky => 0o1000,
            Self::SetGid => 0o2000,
            Self::SetUid => 0o4000,
            Self::Other(o) => *o,
        }
    }

    pub fn other_all() -> Self {
        Self::Other(0o0007)
    }

    pub fn user_all() -> Self {
        Self::Other(0o0700)
    }

    pub fn group_all() -> Self {
        Self::Other(0o0070)
    }

    pub fn user_rw() -> Self {
        Self::Other(0o0600)
    }

    pub fn group_rw() -> Self {
        Self::Other(0o0060)
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct PermissionsAndType(u16);

impl PermissionsAndType {
    pub fn new(typ: InodeType, perms: &[Permission]) -> Self {
        let mut inner = typ.as_u16();
        for perm in perms {
            inner |= perm.as_u16();
        }
        Self(inner)
    }

    pub fn get_raw(&self) -> u16 {
        self.0
    }

    pub fn get_type(&self) -> InodeType {
        match self.0 & 0xf000 {
            0x1000 => InodeType::FiFo,
            0x2000 => InodeType::CharacterDevice,
            0x4000 => InodeType::Directory,
            0x6000 => InodeType::BlockDevice,
            0x8000 => InodeType::File,
            0xa000 => InodeType::Socket,
            other => InodeType::Unknown(other),
        }
    }

    pub fn get_permission(&self, permission: Permission) -> bool {
        (self.0 & permission.as_u16()) > 0
    }

    pub fn set_permission(&mut self, permission: Permission, value: bool) {
        if value {
            self.0 |= permission.as_u16()
        } else {
            self.0 &= !permission.as_u16()
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Inode {
    pub type_and_permission: PermissionsAndType,
    pub uid: u16,
    pub gid: u16,
    pub modification_time: u64,
    pub creation_time: u64,
    pub hardlinks: u16,
    pub block_pointers: [u32; 10],
    pub singly_indirect_block_pointer: u32,
    pub doubly_indirect_block_pointer: u32,
    pub meta: u32,
    padding: [u8; 48],
}

impl Inode {
    pub fn create(
        type_and_permission: PermissionsAndType,
        uid: u16,
        gid: u16,
        now: u64,
        hardlinks: u16,
        meta_data: u32,
    ) -> Self {
        Self {
            block_pointers: [0; 10],
            doubly_indirect_block_pointer: 0,
            singly_indirect_block_pointer: 0,
            creation_time: now,
            modification_time: now,
            meta: meta_data,
            gid,
            uid,
            hardlinks,
            type_and_permission,
            padding: [0; 48],
        }
    }

    fn unallocate_block(
        is_double: bool,
        block_id: u32,
        fs: &mut FileSystem,
    ) -> Result<(), FsError> {
        let block: [u32; 1024] = fs.get_disk().read_struct(FileSystem::pointer(block_id)?)?;

        for ent in block {
            if ent == 0 {
                continue;
            }
            if is_double {
                Self::unallocate_block(false, ent, fs)?;
            }
            fs.free_block(ent)?;
        }

        Ok(())
    }

    fn resize_self(
        &mut self,
        to: u32,
        fs: &mut FileSystem,
        my_inode_addr: u32,
    ) -> Result<(), FsError> {
        let mut blocks_required = to;
        let mut cur_block: u32 = 0;

        loop {
            if let None = self.get_block_id(cur_block, fs) {
                self.get_next_free_block(fs, my_inode_addr)?;
            }
            blocks_required -= 1;
            cur_block += 1;
            if blocks_required == 0 {
                break;
            }
        }

        if cur_block < 10 {
            for i in cur_block..10 {
                if self.block_pointers[i as usize] != 0 {
                    fs.free_block(self.block_pointers[i as usize])?;
                    self.block_pointers[i as usize] = 0;
                }
            }
        }

        if self.singly_indirect_block_pointer != 0 && cur_block >= 10 {
            Self::unallocate_block(false, self.singly_indirect_block_pointer, fs)?;
        }
        if self.doubly_indirect_block_pointer != 0 && cur_block >= 1024 + 10 {
            Self::unallocate_block(true, self.doubly_indirect_block_pointer, fs)?;
        }

        fs.write_inode(my_inode_addr, self)?;

        // TODO: unallocate blocks in singly/dobly indirect block pointers

        Ok(())
    }

    pub fn file_write(
        &mut self,
        buf: &[u8],
        fs: &mut FileSystem,
        my_inode_addr: u32,
    ) -> Result<(), FsError> {
        if self.type_and_permission.get_type() != InodeType::File {
            return Err(FsError::NoSpace);
        }

        let blocks = buf.len().div_ceil(BLOCK_SIZE) as u32;
        self.resize_self(blocks, fs, my_inode_addr)?;

        for i in 0..blocks {
            let block = self.get_block_id(i, fs).ok_or(FsError::NoEntry)?;

            let off = FileSystem::pointer(block)?;
            let start = i as usize * BLOCK_SIZE;
            let end = start + (i as usize * BLOCK_SIZE + 4096).min(buf.len());

            fs.get_disk().write_exact(off, &buf[start..end])?;
        }

        self.meta = (buf.len() % BLOCK_SIZE) as u32;
        fs.write_inode(my_inode_addr, self)?;

        Ok(())
    }

    fn get_block_id(&self, mut index: u32, fs: &mut FileSystem) -> Option<u32> {
        if index < 10 {
            match self.block_pointers[index as usize] {
                0 => None,
                other => Some(other),
            }
        } else if index >= 10 && index < 1034 {
            index -= 10;
            let block_ptr = if self.singly_indirect_block_pointer > 0 {
                self.singly_indirect_block_pointer as usize
            } else {
                return None;
            };
            fs.get_disk()
                .read_struct::<u32>(block_ptr + index as usize * 4)
                .ok()
        } else if index >= 1034 && index < 1024 * 1024 + 10 {
            index -= 10;
            let index_l1 = (index / 1024) as usize;
            let index_l2 = (index % 1024) as usize;

            let block_ptr = if self.doubly_indirect_block_pointer > 0 {
                self.singly_indirect_block_pointer as usize
            } else {
                return None;
            };
            let addr = fs
                .get_disk()
                .read_struct::<u32>(block_ptr + index_l1 * 4)
                .ok()?;

            if addr == 0 {
                return None;
            };
            let addr = fs
                .get_disk()
                .read_struct::<u32>(addr as usize + index_l2 * 4)
                .ok()?;
            if addr == 0 {
                None
            } else {
                Some(addr)
            }
        } else {
            None
        }
    }

    pub fn delete(&mut self, my_inode_addr: u32, fs: &mut FileSystem) -> Result<(), FsError> {
        self.hardlinks -= 1;
        fs.write_inode(my_inode_addr, self)?;
        if self.hardlinks > 0 {
            return Ok(());
        }

        for ptr in self.block_pointers {
            if ptr != 0 {
                fs.free_block(ptr)?;
            }
        }

        if let Ok(singly) = FileSystem::pointer(self.singly_indirect_block_pointer)
            .and_then(|ptr| Ok(fs.get_disk().read_struct::<[u32; 1024]>(ptr)?))
        {
            for s in singly {
                if s != 0 {
                    fs.free_block(s)?;
                }
            }
            fs.free_block(self.singly_indirect_block_pointer)?;
        }

        if let Ok(doubly) = FileSystem::pointer(self.singly_indirect_block_pointer)
            .and_then(|ptr| Ok(fs.get_disk().read_struct::<[u32; 1024]>(ptr)?))
        {
            for s in doubly {
                if let Ok(singlies) = FileSystem::pointer(s)
                    .and_then(|ptr| Ok(fs.get_disk().read_struct::<[u32; 1024]>(ptr)?))
                {
                    for s in singlies {
                        fs.free_block(s)?;
                    }
                    fs.free_block(s)?;
                }
            }
            fs.free_block(self.doubly_indirect_block_pointer)?;
        }

        self.doubly_indirect_block_pointer = 0;
        self.singly_indirect_block_pointer = 0;
        self.block_pointers = [0; 10];

        fs.write_inode(my_inode_addr, self)?;

        let inode_blk_root_addr = my_inode_addr / INODES_PER_BLOCK;

        if let Ok(ptr) = FileSystem::pointer(inode_blk_root_addr) {
            let inodes = fs.get_disk().read_struct::<[Inode; INODES_PER_BLOCK as usize]>(ptr)?;
            let all_free = inodes.iter().map(|f| f.hardlinks == 0).all(|bool| bool);
            if all_free {
                println!("Freeing block {inode_blk_root_addr}");
                fs.free_block(inode_blk_root_addr)?;
                if fs.superblock.earliest_inode_space == inode_blk_root_addr {
                    fs.superblock.earliest_inode_space = 0;
                    fs.write_superblock()?;
                }
            }
        }

        Ok(())
    }

    fn _read(&self, off: usize, buf: &mut [u8], fs: &mut FileSystem) -> Result<usize, FsError> {
        let block_id = off / 4096;
        let block_offset = off % 4096;

        let addr = self
            .get_block_id(block_id as u32, fs)
            .ok_or(FsError::NoEntry)? as usize
            * 4096
            + block_offset;
        Ok(fs.get_disk().read_lossy(addr, buf)?)
    }

    pub fn read_exact(
        &self,
        off: usize,
        buf: &mut [u8],
        fs: &mut FileSystem,
    ) -> Result<(), FsError> {
        if self.read(off, buf, fs)? != buf.len() {
            Err(FsError::NoSpace)
        } else {
            Ok(())
        }
    }

    pub fn read(
        &self,
        mut off: usize,
        buf: &mut [u8],
        fs: &mut FileSystem,
    ) -> Result<usize, FsError> {
        let mut read_already: usize = 0;
        let mut left_to_read = buf.len();

        loop {
            let length = (4096 - off % 4096).min(left_to_read);
            if length == 0 {
                return Ok(read_already);
            }
            let read = self._read(off, &mut buf[read_already..read_already + length], fs)?;
            if read == 0 {
                return Ok(read_already);
            }
            read_already += length;
            left_to_read -= length;
            off += length;
        }
    }

    pub fn read_struct<T>(&mut self, addr: usize, fs: &mut FileSystem) -> Result<T, FsError> {
        let mut c: MaybeUninit<T> = MaybeUninit::uninit();

        if self.read(
            addr,
            unsafe {
                &mut *(core::ptr::slice_from_raw_parts_mut(&mut c as *mut _, size_of::<T>())
                    as *mut [u8])
            },
            fs,
        )? != size_of::<T>()
        {
            Err(FsError::DiskError(DiskError::NotEnoughSpace))
        } else {
            Ok(unsafe { c.assume_init() })
        }
    }

    pub fn write_dir_entry(
        &mut self,
        fs: &mut FileSystem,
        dir_entry: &DirEntry,
        entry_nbr: Option<u32>,
        my_inode_addr: u32,
    ) -> Result<u32, FsError> {
        if self.type_and_permission.get_type() != InodeType::Directory {
            return Err(FsError::NoEntry);
        }

        let (blk_id, off, entry_nbr) = match entry_nbr {
            Some(v) => self.get_dir_entry_by_nbr(fs, v)?,
            None => self.get_next_free_dir_entry_slot(fs, my_inode_addr)?,
        };

        let addr = self.get_block_id(blk_id, fs).ok_or(FsError::NoEntry)?;

        dir_entry.write_to_disk(fs.get_disk(), addr as usize * BLOCK_SIZE + off as usize)?;

        Ok(entry_nbr)
    }

    fn get_dir_entry_by_nbr(
        &mut self,
        fs: &mut FileSystem,
        block_id: u32,
    ) -> Result<(u32, u32, u32), FsError> {
        let mut blk_id = 0;
        let mut off: u32 = 0;
        let mut slot_id: u32 = 0;

        loop {
            let block = self.get_block_id(blk_id, fs);
            match block {
                None => return Err(FsError::NoEntry),
                Some(v) => {
                    let dir_entry = fs
                        .get_disk()
                        .read_struct::<DirEntry>(v as usize * BLOCK_SIZE + off as usize)?;
                    if slot_id == block_id {
                        return Ok((blk_id, off, slot_id));
                    }

                    off += dir_entry.get_size();
                    if off >= 3796 {
                        // dir_entry wouldnt fit in this block anymore
                        blk_id += 1;
                        off = 0;
                    }
                    slot_id += 1;
                }
            }
        }
    }

    fn get_next_free_block(
        &mut self,
        fs: &mut FileSystem,
        my_inode_addr: u32,
    ) -> Result<u32, FsError> {
        let mut blk_id: u32 = 0;
        loop {
            if let None = self.get_block_id(blk_id, fs) {
                break;
            }
            blk_id += 1;
        }

        if blk_id < 10 {
            let blk = fs.allocate_block(false)?;
            self.block_pointers[blk_id as usize] = blk;
            fs.write_inode(my_inode_addr, &self)?;
        } else if blk_id >= 10 && blk_id < 1024 + 10 {
            if self.singly_indirect_block_pointer == 0 {
                self.singly_indirect_block_pointer = fs.allocate_block(false)?;
                fs.write_inode(my_inode_addr, &self)?;
            }
            let blk = fs.allocate_block(false)?;
            fs.get_disk().write_struct(
                self.singly_indirect_block_pointer as usize + (blk_id as usize - 10) * 4,
                &blk,
            )?;
        } else if blk_id >= 1024 + 10 && blk_id < 1024 * 1024 + 10 {
            if self.doubly_indirect_block_pointer == 0 {
                self.doubly_indirect_block_pointer = fs.allocate_block(false)?;
                fs.write_inode(my_inode_addr, &self)?;
            }
            let singly_blk_ptr = fs.allocate_block(false)?;
            fs.get_disk().write_struct(
                self.doubly_indirect_block_pointer as usize + ((blk_id as usize - 10) / 1024 * 4),
                &singly_blk_ptr,
            )?;
            let blk = fs.allocate_block(false)?;
            fs.get_disk().write_struct(
                singly_blk_ptr as usize + ((blk_id as usize - 10) % 1024 * 4),
                &blk,
            )?;
        } else {
            return Err(FsError::DiskError(DiskError::NotEnoughSpace));
        }

        Ok(blk_id)
    }

    fn get_next_free_dir_entry_slot(
        &mut self,
        fs: &mut FileSystem,
        my_inode_addr: u32,
    ) -> Result<(u32, u32, u32), FsError> {
        let mut blk_id = 0;
        let mut off: u32 = 0;
        let mut slot_id: u32 = 0;

        loop {
            let block = self.get_block_id(blk_id, fs);
            match block {
                None => {
                    blk_id = self.get_next_free_block(fs, my_inode_addr)?;
                    continue;
                }
                Some(v) => {
                    let dir_entry = fs
                        .get_disk()
                        .read_struct::<DirEntry>(v as usize * BLOCK_SIZE + off as usize)?;
                    if dir_entry.inode == 0 || dir_entry.is_empty() {
                        return Ok((blk_id, off, slot_id));
                    } else {
                        off += dir_entry.get_size();
                        if off >= 3796 {
                            // dir_entry wouldnt fit in this block anymore
                            blk_id += 1;
                            off = 0;
                        }
                        slot_id += 1;
                    }
                }
            }
        }
    }
}
