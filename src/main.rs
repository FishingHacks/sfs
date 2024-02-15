use std::{fs::File, path::Path};

use disk::Disk;
use fs::{FileSystem, FsError, BLOCK_SIZE};

use crate::{
    directory::DirectoryIterator, fs::INODES_PER_BLOCK, inode::{Inode, InodeType, Permission, PermissionsAndType}
};

mod directory;
mod disk;
mod fs;
mod inode;
mod superblock;

fn main() {
    // let mut fs: FileSystem = File::options()
    //     .read(true)
    //     .write(true)
    //     .open("fs.img")
    //     .map(|f| {
    //         FileSystem::from_disk(Disk::new(Box::new(f)))
    //             .expect("Failed to create fs from disk image")
    //     })
    //     .unwrap_or_else(|_| write_empty_fs_to_file(300, "My Filesystem", "fs.img"));
    let mut fs = FileSystem::create(300, "My Filesystem").expect("Failed to create empty fs");

    println!("got fs with name: {}", fs.superblock.get_name());

    let mut nodes = vec![];

    for i in 0..INODES_PER_BLOCK {
        nodes.push(fs.create_dir_entry(
            fs.superblock.root_inode,
            Inode::create(
                PermissionsAndType::new(
                    InodeType::File,
                    &[
                        Permission::user_rw(),
                        Permission::group_rw(),
                        Permission::OtherRead,
                    ],
                ),
                0,
                0,
                0,
                0,
                0,
            ),
            format!("my_file_{i}"),
        ).expect("Failed to create directory entry"));
    }


    for node in nodes {
        fs.read_inode(node).unwrap().delete(node, &mut fs).unwrap();
    }

    let node = fs
        .read_inode(fs.superblock.root_inode)
        .expect("Failed to read /");

    for dir_entry in DirectoryIterator::new(node, &mut fs) {
        println!("listing {:?}: {}", dir_entry.get_name(), dir_entry.inode);
    }
}

fn write_empty_fs_to_file<P: AsRef<Path>>(num_blocks: u32, name: &str, path: P) -> FileSystem {
    let mut fs = FileSystem::create(num_blocks, name).expect("Failed to create empty fs");
    let mut f = File::options()
        .write(true)
        .create(true)
        .open(&path)
        .expect("Failed to create file");
    fs.get_disk()
        .duplicate(&mut f)
        .expect("Failed to duplicate disk");
    drop(f);
    drop(fs);

    FileSystem::from_disk(Disk::new(Box::new(
        File::options()
            .read(true)
            .write(true)
            .open(path)
            .expect("Failed to read newly created file"),
    )))
    .expect("Failed to create empty fs")
}

pub fn read_entire_inode(inode: &mut Inode, fs: &mut FileSystem) -> Result<Vec<u8>, FsError> {
    let mut vec = Vec::with_capacity(BLOCK_SIZE);

    let mut block = [0; BLOCK_SIZE];
    let mut off = 0;
    loop {
        let read = match inode.read(off, &mut block, fs) {
            Ok(v) => v,
            Err(FsError::NoEntry) => 0,
            e => e?,
        };
        
        vec.extend(&block[0..read]);

        if read != BLOCK_SIZE {
            break;
        }

        off += BLOCK_SIZE;
    }

    for _ in 0..(4096 - inode.meta) {
        vec.pop();
    }

    Ok(vec)
}