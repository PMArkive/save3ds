use crate::error::*;
use crate::random_access_file::*;
use std::cell::Cell;
use std::rc::Rc;

pub struct DualFile {
    selector: Rc<RandomAccessFile>,
    pair: [Rc<RandomAccessFile>; 2],
    modified: Cell<u8>,
    len: usize,
}

impl DualFile {
    pub fn new(
        selector: Rc<RandomAccessFile>,
        pair: [Rc<RandomAccessFile>; 2],
    ) -> Result<DualFile, Error> {
        let len = pair[0].len();
        if pair[1].len() != len {
            return make_error(Error::SizeMismatch);
        }
        if selector.len() != 1 {
            return make_error(Error::SizeMismatch);
        }
        Ok(DualFile {
            selector,
            pair,
            modified: Cell::new(0),
            len,
        })
    }
}

impl RandomAccessFile for DualFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        if pos + buf.len() > self.len {
            return make_error(Error::OutOfBound);
        }
        let mut select = [0; 1];
        self.selector.read(0, &mut select)?;
        select[0] ^= self.modified.get();
        self.pair[select[0] as usize].read(pos, buf)
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len {
            return make_error(Error::OutOfBound);
        }
        let mut select = [0; 1];
        self.selector.read(0, &mut select)?;
        let prev = select[0] as usize;
        let cur = 1 - prev;
        self.pair[cur].write(pos, buf)?;
        if self.modified.get() == 0 {
            if pos != 0 {
                let mut edge_buf = vec![0; pos];
                self.pair[prev].read(0, &mut edge_buf)?;
                self.pair[cur].write(0, &edge_buf)?;
            }
            if end != self.len {
                let mut edge_buf = vec![0; self.len - end];
                self.pair[prev].read(end, &mut edge_buf)?;
                self.pair[cur].write(end, &edge_buf)?;
            }
            self.modified.set(1);
        }
        Ok(())
    }
    fn len(&self) -> usize {
        self.len
    }
    fn commit(&self) -> Result<(), Error> {
        if self.modified.get() == 1 {
            let mut select = [0; 1];
            self.selector.read(0, &mut select)?;
            select[0] = 1 - select[0];
            self.selector.write(0, &select)?;
            self.modified.set(0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::dual_file::DualFile;
    use crate::memory_file::MemoryFile;
    use crate::random_access_file::*;
    use std::rc::Rc;

    #[test]
    fn fuzz() {
        use rand::distributions::Standard;
        use rand::prelude::*;

        let mut rng = rand::thread_rng();
        for _ in 0..10 {
            let len = rng.gen_range(1, 10_000);
            let selector = Rc::new(MemoryFile::new(vec![0; 1]));
            let pair: [Rc<RandomAccessFile>; 2] = [
                Rc::new(MemoryFile::new(
                    rng.sample_iter(&Standard).take(len).collect(),
                )),
                Rc::new(MemoryFile::new(
                    rng.sample_iter(&Standard).take(len).collect(),
                )),
            ];
            let init: Vec<u8> = rng.sample_iter(&Standard).take(len).collect();
            let mut dpfs_level = DualFile::new(selector.clone(), pair.clone()).unwrap();
            dpfs_level.write(0, &init).unwrap();
            let plain = MemoryFile::new(init);

            for _ in 0..1000 {
                let operation = rng.gen_range(1, 10);
                if operation == 1 {
                    dpfs_level.commit().unwrap();
                    dpfs_level = DualFile::new(selector.clone(), pair.clone()).unwrap();
                } else if operation < 4 {
                    dpfs_level.commit().unwrap();
                } else {
                    let pos = rng.gen_range(0, len);
                    let data_len = rng.gen_range(1, len - pos + 1);
                    if operation < 7 {
                        let mut a = vec![0; data_len];
                        let mut b = vec![0; data_len];
                        dpfs_level.read(pos, &mut a).unwrap();
                        plain.read(pos, &mut b).unwrap();
                        assert_eq!(a, b);
                    } else {
                        let a: Vec<u8> = rng.sample_iter(&Standard).take(data_len).collect();
                        dpfs_level.write(pos, &a).unwrap();
                        plain.write(pos, &a).unwrap();
                    }
                }
            }
        }
    }
}