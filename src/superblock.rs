use std::time::{SystemTime, UNIX_EPOCH};

use crate::{disk::Disk, fs::{FsError, BLOCKS_PER_BLOCKARRAY}};


#[repr(C)]
#[derive(Debug, Clone)]
pub struct Superblock {
    signature: [u8; 8],
    pub earliest_free: u32,
    pub earliest_inode_space: u32,
    pub last_free: u32,
    pub total_unused: u32,
    pub total_blocks: u32,
    pub last_mount: u64,
    pub last_write: u64,
    pub name: [u8; 32],
    pub file_prealloc: u8,
    pub dir_prealloc: u8,
    pub root_inode: u32,
}

pub const SUPERBLOCK_SIGNATURE_SFS: &[u8; 8] = b"SFs sblk";

impl Superblock {
    pub fn read(disk: &mut Disk, addr: usize) -> Result<Self, FsError> {
        let sblk = disk.read_struct::<Self>(addr)?;
        if sblk.signature != *SUPERBLOCK_SIGNATURE_SFS {
            Err(FsError::InvalidSignature)
        } else {
            Ok(sblk)
        }
    }

    pub fn total_used(&self) -> u32 {
        self.total_blocks - self.total_unused
    }

    pub fn get_name<'a>(&'a self) -> String {
        let mut str = String::with_capacity(32);

        for i in 0..32 {
            if self.name[i] == 0 {
                break;
            } else {
                str.push(self.name[i] as char);
            }
        }

        str
    }

    pub fn new(name: &str, num_blocks: u32) -> Result<Self, FsError> {
        let mut name_slice = [0_u8; 32];
        for (i, byte) in name.bytes().enumerate() {
            if i >= 32 {
                return Err(FsError::NameTooLong);
            }
            name_slice[i] = byte;
        }

        Ok(Self {
            name: name_slice,
            signature: *SUPERBLOCK_SIGNATURE_SFS,
            dir_prealloc: 1,
            file_prealloc: 1,
            last_free: num_blocks - 1,
            earliest_free: 2,
            earliest_inode_space: 0,
            last_mount: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards ftw")
                .as_secs(),
            last_write: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards ftw")
                .as_secs(),
            total_blocks: num_blocks,
            total_unused: num_blocks - 1 - num_blocks.div_ceil(BLOCKS_PER_BLOCKARRAY),
            root_inode: 0, // the FileSystem::new(...) handles this
        })
    }
}