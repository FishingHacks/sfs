use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    directory::DirEntry,
    disk::{Disk, DiskError},
    inode::{Inode, InodeType, Permission, PermissionsAndType},
    superblock::Superblock,
};

#[derive(Debug)]
pub enum FsError {
    DiskError(DiskError),
    InvalidSignature,
    NameTooLong,
    InvalidBlock,
    NoEntry,
    NoSpace,
    FailSuperblockWrite,
}

impl From<DiskError> for FsError {
    fn from(value: DiskError) -> Self {
        Self::DiskError(value)
    }
}

#[derive(Debug)]
pub struct FileSystem {
    pub superblock: Superblock,
    disk: Disk,
}

pub const BLOCKS_PER_BLOCKARRAY: u32 = 2048 * 8;

#[repr(C)]
pub struct BlockArrayDescriptor<'a>(&'a mut Disk, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockArrayEntry {
    BlockArrayDescriptor,
    Unused,
    InodeBlock,
    Allocated,
}

impl<'a> BlockArrayDescriptor<'a> {
    pub fn from_disk(disk: &'a mut Disk, idx: u32) -> Self {
        Self(disk, idx)
    }

    pub fn create(disk: &'a mut Disk, idx: u32) -> Result<Self, DiskError> {
        let mut value = Self(disk, idx);
        value.set(0, BlockArrayEntry::BlockArrayDescriptor)?;
        Ok(value)
    }

    pub fn get(&mut self, index: u32) -> Result<BlockArrayEntry, DiskError> {
        if index == 0 {
            return Ok(BlockArrayEntry::BlockArrayDescriptor);
        }

        let block_index = (index / 8) as usize;
        let bitmap_offset = index % 8;

        if self
            .0
            .read_struct::<u8>(block_index + (self.1 as usize * BLOCKS_PER_BLOCKARRAY as usize))?
            & (1 << bitmap_offset)
            == 0
        {
            Ok(BlockArrayEntry::Unused)
        } else if self.0.read_struct::<u8>(
            block_index + (self.1 as usize * BLOCKS_PER_BLOCKARRAY as usize) + 2048,
        )? & (1 << bitmap_offset)
            > 0
        {
            Ok(BlockArrayEntry::InodeBlock)
        } else {
            Ok(BlockArrayEntry::Allocated)
        }
    }

    pub fn set(&mut self, index: u32, mut typ: BlockArrayEntry) -> Result<(), DiskError> {
        if index >= BLOCKS_PER_BLOCKARRAY {
            return Ok(());
        }

        if index == 0 {
            typ = BlockArrayEntry::BlockArrayDescriptor;
        } else if index != 0 && typ == BlockArrayEntry::BlockArrayDescriptor {
            typ = BlockArrayEntry::Allocated;
        }

        let block_index = (index / 8) as usize + (self.1 as usize * BLOCKS_PER_BLOCKARRAY as usize);
        let bitmap_offset = index % 8;

        let mut usage_bitmap = self.0.read_struct::<u8>(block_index)?;
        let mut type_bitmap = self.0.read_struct::<u8>(block_index + 2048)?;

        if typ != BlockArrayEntry::Unused {
            usage_bitmap |= 1 << bitmap_offset;
        } else {
            usage_bitmap &= !(1 << bitmap_offset);
        }

        if typ == BlockArrayEntry::InodeBlock {
            type_bitmap |= 1 << bitmap_offset;
        } else {
            type_bitmap &= !(1 << bitmap_offset);
        }

        self.0.write_struct(block_index, &usage_bitmap)?;
        self.0.write_struct(block_index + 2048, &type_bitmap)?;

        Ok(())
    }
}

pub const INODE_SIZE: usize = 128;
pub const BLOCK_SIZE: usize = 4096;
pub const INODES_PER_BLOCK: u32 = (BLOCK_SIZE / INODE_SIZE) as u32; // block size / inode size

impl FileSystem {
    pub fn from_disk(mut disk: Disk) -> Result<Self, FsError> {
        let superblock = Superblock::read(&mut disk, 4096 /* block #1 */)?;
        Ok(Self { disk, superblock })
    }

    pub fn get_disk<'a>(&'a mut self) -> &'a mut Disk {
        &mut self.disk
    }

    pub fn pointer(block_id: u32) -> Result<usize, FsError> {
        if block_id % BLOCKS_PER_BLOCKARRAY == 0 {
            Err(FsError::InvalidBlock)
        } else {
            Ok(block_id as usize * BLOCK_SIZE)
        }
    }

    pub fn read_inode(&mut self, inode_nbr: u32) -> Result<Inode, FsError> {
        Ok(self.disk.read_struct(inode_nbr as usize * 128)?)
    }

    pub fn write_inode(&mut self, inode_nbr: u32, inode: &Inode) -> Result<(), FsError> {
        self.disk.write_struct(inode_nbr as usize * 128, inode)?;
        Ok(())
    }

    fn get_inode_physical(&mut self) -> Result<usize, FsError> {
        // if self.superblock.earliest_inode_space == 0 {
        //     self.superblock.earliest_inode_space = self.allocate_block(true)?;
        // }
        let inode_addr = self.superblock.earliest_inode_space as usize * INODE_SIZE;

        if inode_addr != 0 {
            for i in 0..INODES_PER_BLOCK {
                let inode = self
                    .disk
                    .read_struct::<Inode>(inode_addr + i as usize * INODE_SIZE)?;
                if inode.hardlinks == 0 {
                    return Ok(inode_addr + i as usize * INODE_SIZE);
                }
            }
        }
        let block = self.allocate_block(true)?;
        return Ok(Self::pointer(block)?);
    }

    pub fn write_superblock(&mut self) -> Result<(), FsError> {
        match self
            .disk
            .write_struct(4096 /* block #1 */, &self.superblock)
        {
            Err(..) => Err(FsError::FailSuperblockWrite),
            Ok(..) => Ok(()),
        }
    }

    pub fn create_dir_entry(
        &mut self,
        parent_nbr: u32,
        mut child: Inode,
        name: String,
    ) -> Result<u32, FsError> {
        child.hardlinks = 0;
        let child_nbr = self.create_inode(&child)?;
        self.link_to_inode(parent_nbr, child_nbr, name)
    }

    pub fn link_to_inode(
        &mut self,
        parent_nbr: u32,
        child_nbr: u32,
        name: String,
    ) -> Result<u32, FsError> {
        let mut node = self.read_inode(child_nbr)?;
        node.hardlinks += 1;
        self.write_inode(child_nbr, &node)?;

        let mut node = self.read_inode(parent_nbr)?;
        node.write_dir_entry(self, &DirEntry::create(child_nbr, name)?, None, parent_nbr)?;
        Ok(child_nbr)
    }

    fn clear_block(&mut self, blk_id: u32) -> Result<(), FsError> {
        let space = [0; BLOCK_SIZE];
        self.disk.write_exact(Self::pointer(blk_id)?, &space)?;
        Ok(())
    }

    pub fn free_block(&mut self, block_id: u32) -> Result<(), FsError> {
        if block_id == 0 {
            return Err(FsError::InvalidBlock);
        }
        if self.superblock.earliest_free > block_id {
            self.superblock.earliest_free = block_id;
            self.write_superblock()?;
        }

        BlockArrayDescriptor::from_disk(&mut self.disk, block_id / BLOCKS_PER_BLOCKARRAY)
            .set(block_id % BLOCKS_PER_BLOCKARRAY, BlockArrayEntry::Unused)?;
        self.clear_block(block_id)?;

        Ok(())
    }

    pub fn allocate_block(&mut self, for_inodes: bool) -> Result<u32, FsError> {
        let blk = self.superblock.earliest_free;
        if blk == 0 {
            return Err(FsError::NoSpace);
        } else if blk == self.superblock.last_free {
            self.superblock.last_free = 0;
        }

        self.superblock.earliest_free = 0;
        BlockArrayDescriptor::from_disk(&mut self.disk, blk / BLOCKS_PER_BLOCKARRAY).set(
            blk % BLOCKS_PER_BLOCKARRAY,
            if for_inodes {
                BlockArrayEntry::InodeBlock
            } else {
                BlockArrayEntry::Allocated
            },
        )?;

        for i in blk + 1..self.superblock.total_blocks {
            if BlockArrayDescriptor::from_disk(&mut self.disk, i / BLOCKS_PER_BLOCKARRAY)
                .get(i % BLOCKS_PER_BLOCKARRAY)?
                == BlockArrayEntry::Unused
            {
                self.superblock.earliest_free = i;
                if for_inodes {
                    self.superblock.earliest_inode_space = blk * INODES_PER_BLOCK;
                }
                self.write_superblock()?;
                self.clear_block(blk)?;
                return Ok(blk);
            }
        }

        self.write_superblock()?;
        Err(FsError::NoSpace)
    }

    pub fn create_inode(&mut self, inode: &Inode) -> Result<u32, FsError> {
        let addr = (self.get_inode_physical()? / INODE_SIZE) as u32;
        self.write_inode(addr, inode)?;
        Ok(addr)
    }

    pub fn create(num_blocks: u32, fs_name: &str) -> Result<Self, FsError> {
        let mut disk = Disk::new_virtual(num_blocks);

        if num_blocks < 3 {
            return Err(FsError::DiskError(DiskError::NotEnoughSpace));
        }

        let superblock = Superblock::new(fs_name, num_blocks)?;
        disk.write_struct(4096 /* block */, &superblock)?;

        for i in 0..num_blocks.div_ceil(BLOCKS_PER_BLOCKARRAY) {
            println!("writing block array {i}");
            let mut blk_arr = BlockArrayDescriptor::create(&mut disk, i)?;
            if i == 0 {
                blk_arr.set(1, BlockArrayEntry::Allocated)?;
            }
        }

        let mut fs = Self { superblock, disk };

        let inode = Inode::create(
            PermissionsAndType::new(
                InodeType::Directory,
                &[
                    Permission::group_all(),
                    Permission::user_all(),
                    Permission::OtherRead,
                    Permission::OtherExecute,
                ],
            ),
            0,
            0,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards ftw")
                .as_secs(),
            1,
            0,
        );

        fs.superblock.root_inode = fs.create_inode(&inode)?;
        fs.write_superblock()?;

        Ok(fs)
    }
}
