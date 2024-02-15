use std::mem::size_of;

use crate::{
    disk::Disk,
    fs::{FileSystem, FsError, BLOCK_SIZE},
    inode::Inode,
};

pub const DIRENTRY_NAME_LENGTH: usize = 0xff;

#[derive(Debug)]
#[repr(C)]
pub struct DirEntry {
    name_size: u8,
    pub inode: u32,
    name: [u8; DIRENTRY_NAME_LENGTH],
}

impl DirEntry {
    pub fn read_from_disk(
        inode: &mut Inode,
        fs: &mut FileSystem,
        addr: usize,
    ) -> Result<Self, FsError> {
        let mut empty = Self {
            name_size: 0,
            inode: 0,
            name: [0; DIRENTRY_NAME_LENGTH],
        };

        let mut value: [u8; 1] = [0];

        inode.read_exact(addr, &mut value, fs)?;
        empty.name_size = value[0];

        empty.inode = inode.read_struct::<u32>(addr + 1, fs)?;

        if empty.name_size != 0 {
            inode.read_exact(addr + 5, &mut empty.name[0..empty.name_size as usize], fs)?;
        }

        Ok(empty)
    }

    pub fn create(inode: u32, name: String) -> Result<Self, FsError> {
        if name.as_bytes().len() >= DIRENTRY_NAME_LENGTH || name.is_empty() {
            return Err(FsError::NameTooLong);
        }

        let mut ent = DirEntry {
            name_size: name.len() as u8,
            inode,
            name: [0; DIRENTRY_NAME_LENGTH],
        };

        for (i, c) in name.bytes().enumerate() {
            ent.name[i] = c;
        }

        Ok(ent)
    }

    pub fn is_empty(&self) -> bool {
        self.inode == 0 || self.name_size == 0
    }

    pub fn get_size(&self) -> u32 {
        5 + self.name_size as u32
    }

    pub fn write_to_disk(&self, disk: &mut Disk, addr: usize) -> Result<(), FsError> {
        disk.write_exact(addr, &[self.name_size])?;
        disk.write_struct(addr + 1, &self.inode)?;
        disk.write_exact(addr + 5, &self.name[0..self.name_size as usize])?;
        Ok(())
    }

    pub fn get_name(&self) -> String {
        String::from_utf8_lossy(&self.name[0..self.name_size as usize]).to_string()
    }
}

pub struct DirectoryIterator<'a> {
    next_off: u32,
    next_blk: u32,
    inode: Inode,
    fs: &'a mut FileSystem,
}

impl<'a> DirectoryIterator<'a> {
    pub fn new(inode: Inode, fs: &'a mut FileSystem) -> Self {
        Self {
            fs,
            inode,
            next_blk: 0,
            next_off: 0,
        }
    }
}

impl Iterator for DirectoryIterator<'_> {
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let dir_entry = DirEntry::read_from_disk(
            &mut self.inode,
            &mut self.fs,
            self.next_blk as usize * BLOCK_SIZE + self.next_off as usize,
        )
        .ok()?;

        self.next_off += dir_entry.get_size();
        if self.next_off + size_of::<DirEntry>() as u32 >= BLOCK_SIZE as u32 {
            self.next_off = 0;
            self.next_blk += 1;
        }
        if dir_entry.is_empty() {
            return self.next();
        }

        self.next_off += dir_entry.get_size();
        if self.next_off + size_of::<DirEntry>() as u32 >= BLOCK_SIZE as u32 {
            self.next_off = 0;
            self.next_blk += 1;
        }
        Some(dir_entry)
    }
}
