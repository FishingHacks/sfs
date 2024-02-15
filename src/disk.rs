use std::{
    fmt::Debug, fs::File, io::ErrorKind, mem::{size_of, MaybeUninit}, os::unix::fs::FileExt
};

use crate::fs::BLOCK_SIZE;

#[derive(Debug)]
pub enum DiskError {
    NotEnoughSpace,
    GenericError,
}

pub trait IO {
    fn read_lossy(&mut self, addr: usize, buf: &mut [u8]) -> Result<usize, DiskError>;
    fn write_lossy(&mut self, addr: usize, buf: &[u8]) -> Result<usize, DiskError>;

    fn read_exact(&mut self, addr: usize, buf: &mut [u8]) -> Result<(), DiskError> {
        if self.read_lossy(addr, buf)? != buf.len() {
            Err(DiskError::NotEnoughSpace)
        } else {
            Ok(())
        }
    }
    fn write_exact(&mut self, addr: usize, buf: &[u8]) -> Result<(), DiskError> {
        if self.write_lossy(addr, buf)? != buf.len() {
            Err(DiskError::NotEnoughSpace)
        } else {
            Ok(())
        }
    }
}

pub struct Disk(Box<dyn IO>);

impl Debug for Disk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Disk")
    }
}
impl Disk {
    pub fn new(io: Box<dyn IO>) -> Self {
        Self(io)
    }

    pub fn read_struct<T>(&mut self, addr: usize) -> Result<T, DiskError> {
        let mut c: MaybeUninit<T> = core::mem::MaybeUninit::uninit();

        self.0.read_exact(addr, unsafe {
            &mut *(core::ptr::slice_from_raw_parts_mut(&mut c as *mut _, size_of::<T>())
                as *mut [u8])
        })?;

        unsafe { Ok(c.assume_init()) }
    }

    pub fn write_struct<T>(&mut self, addr: usize, structure: &T) -> Result<(), DiskError> {
        self.0.write_exact(addr, unsafe {
            &*(core::ptr::slice_from_raw_parts(structure as *const _, size_of::<T>())
                as *mut [u8])
        })
    }

    pub fn read_lossy(&mut self, addr: usize, buf: &mut [u8]) -> Result<usize, DiskError> {
        self.0.read_lossy(addr, buf)
    }
    pub fn write_lossy(&mut self, addr: usize, buf: &[u8]) -> Result<usize, DiskError> {
        self.0.write_lossy(addr, buf)
    }
    pub fn read_exact(&mut self, addr: usize, buf: &mut [u8]) -> Result<(), DiskError> {
        self.0.read_exact(addr, buf)
    }
    pub fn write_exact(&mut self, addr: usize, buf: &[u8]) -> Result<(), DiskError> {
        self.0.write_exact(addr, buf)
    }

    pub fn new_virtual(blocks: u32) -> Self {
        let mut vec = Vec::new();
        vec.resize(blocks as usize * 4096, 0);
        Self(Box::new(vec))
    }

    pub fn to_vec(&mut self) -> Result<Vec<u8>, DiskError> {
        let mut vec = Vec::new();
        let mut block: [u8; 4096] = [0; 4096];
        let mut addr: usize = 0;

        loop {
            let read = self.read_lossy(addr, &mut block)?;
            if read == 0 {
                return Ok(vec);
            }

            vec.extend(&block[0..read]);
            addr += read;
        }
    }

    /// Errors when other could not be written to while self has more data
    pub fn duplicate(&mut self, other: &mut dyn IO) -> Result<usize, DiskError> {
        let mut block: [u8; 4096] = [0; 4096];
        let mut addr: usize = 0;

        loop {
            let read = self.read_lossy(addr, &mut block)?;
            if read == 0 {
                return Ok(addr);
            }

            other.write_exact(addr, &block)?;
            addr += read;
        }
    }
}

impl IO for Vec<u8> {
    fn read_lossy(&mut self, addr: usize, buf: &mut [u8]) -> Result<usize, DiskError> {
        // let blk_1 = addr / BLOCK_SIZE;
        // let blk_2 = (addr+buf.len() - 1) / BLOCK_SIZE;
        // println!("Reading {}..{} (blk {}..{})", addr, addr+buf.len(), blk_1, blk_2);
        for i in 0..buf.len() {
            if let Some(v) = self.get(i + addr) {
                buf[i] = *v;
            } else {
                return Ok(i); // the last index we could read is i-1, and length is last_index+1, so i is the length of what we've read
            }
        }
        Ok(buf.len())
    }

    fn write_lossy(&mut self, addr: usize, buf: &[u8]) -> Result<usize, DiskError> {
        // let blk_1 = addr / BLOCK_SIZE;
        // let blk_2 = (addr+buf.len() - 1) / BLOCK_SIZE;
        // println!("Writing {}..{} (blk {}..{})", addr, addr+buf.len(), blk_1, blk_2);

        for i in 0..buf.len() {
            if addr + i >= self.len() {
                return Ok(i); // the last index we could write is i-1, and length is last_index+1, so i is the length of what we've written
            } else {
                self[addr + i] = buf[i];
            }
        }
        Ok(buf.len())
    }
}

impl IO for File {
    fn read_lossy(&mut self, addr: usize, buf: &mut [u8]) -> Result<usize, DiskError> {
        match self.read_at(buf, addr as u64) {
            Ok(v) => Ok(v),
            Err(e) => match e.kind() {
                ErrorKind::AddrNotAvailable => Ok(0),
                ErrorKind::WriteZero => Ok(0),
                _ => Err(DiskError::GenericError),
            },
        }
    }

    fn write_lossy(&mut self, addr: usize, buf: &[u8]) -> Result<usize, DiskError> {
        match self.write_at(buf, addr as u64) {
            Ok(v) => Ok(v),
            Err(e) => match e.kind() {
                ErrorKind::AddrNotAvailable => Ok(0),
                ErrorKind::WriteZero => Ok(0),
                _ => Err(DiskError::GenericError),
            },
        }
    }
}
