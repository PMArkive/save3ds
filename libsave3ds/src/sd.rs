use crate::aes_ctr_file::AesCtrFile;
use crate::disk_file::DiskFile;
use crate::error::*;
use crate::key_engine::*;
use crate::misc::*;
use crate::random_access_file::*;
use crate::sd_nand_common::*;
use sha2::*;
use std::path::*;
use std::rc::Rc;

pub struct Sd {
    path: PathBuf,
    key: [u8; 16],
}

impl Sd {
    pub fn new(sd_path: &str, key_x: [u8; 16], key_y: [u8; 16]) -> Result<Sd, Error> {
        let path = std::fs::read_dir(
            PathBuf::from(sd_path)
                .join("Nintendo 3DS")
                .join(hash_movable(key_y)),
        )?
        .find(|a| {
            a.as_ref()
                .map(|a| a.file_type().map(|a| a.is_dir()).unwrap_or(false))
                .unwrap_or(false)
        })
        .ok_or(Error::BrokenSd)??
        .path();
        let key = scramble(key_x, key_y);
        Ok(Sd { path, key })
    }
}

impl SdNandFileSystem for Sd {
    fn open(&self, path: &[&str], write: bool) -> Result<Rc<dyn RandomAccessFile>, Error> {
        let file_path = path.iter().fold(self.path.clone(), |a, b| a.join(b));
        let file = Rc::new(DiskFile::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(file_path)?,
        )?);

        let hash_path: Vec<u8> = path
            .iter()
            .flat_map(|s| std::iter::once(b'/').chain(s.bytes()))
            .chain(std::iter::once(0))
            .flat_map(|c| std::iter::once(c).chain(std::iter::once(0)))
            .collect();

        let mut hasher = Sha256::new();
        hasher.update(&hash_path);
        let hash = hasher.finalize();
        let mut ctr = [0; 16];
        for (i, c) in ctr.iter_mut().enumerate() {
            *c = hash[i] ^ hash[i + 16];
        }

        Ok(Rc::new(AesCtrFile::new(file, self.key, ctr, false)))
    }

    fn create(&self, path: &[&str], len: usize) -> Result<(), Error> {
        let file_path = path.iter().fold(self.path.clone(), |a, b| a.join(b));
        std::fs::create_dir_all(file_path.parent().unwrap())?;
        let f = std::fs::File::create(file_path)?;
        f.set_len(len as u64)?;
        Ok(())
    }

    fn remove(&self, path: &[&str]) -> Result<(), Error> {
        let file_path = path.iter().fold(self.path.clone(), |a, b| a.join(b));
        std::fs::remove_file(file_path)?;
        Ok(())
    }

    fn remove_dir(&self, path: &[&str]) -> Result<(), Error> {
        let dir_path = path.iter().fold(self.path.clone(), |a, b| a.join(b));
        if dir_path.exists() {
            std::fs::remove_dir_all(dir_path)?;
        }
        Ok(())
    }
}
