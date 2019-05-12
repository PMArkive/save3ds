use fuse::*;
use getopts::*;
use libc::{EBADF, EEXIST, EIO, EISDIR, ENOENT, ENOSPC, ENOTDIR, ENOTEMPTY, EROFS};
use libsave3ds::error::*;
use libsave3ds::save_data::*;
use libsave3ds::Resource;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::rc::Rc;
use time;

struct SaveDataFilesystem {
    save: Rc<SaveData>,
    fh_map: HashMap<u64, File>,
    next_fh: u64,
    read_only: bool,
}

impl SaveDataFilesystem {
    fn new(save: Rc<SaveData>, read_only: bool) -> SaveDataFilesystem {
        SaveDataFilesystem {
            save,
            fh_map: HashMap::new(),
            next_fh: 1,
            read_only,
        }
    }

    fn make_dir_attr(&self, ino: u64, sub_file_count: usize) -> FileAttr {
        FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: time::Timespec::new(0, 0),
            mtime: time::Timespec::new(0, 0),
            ctime: time::Timespec::new(0, 0),
            crtime: time::Timespec::new(0, 0),
            kind: FileType::Directory,
            perm: if self.read_only { 0o555 } else { 0o777 },
            nlink: 2 + sub_file_count as u32,
            uid: 501,
            gid: 20,
            rdev: 0,
            flags: 0,
        }
    }

    fn make_file_attr(&self, ino: u64, file_size: usize) -> FileAttr {
        FileAttr {
            ino,
            size: file_size as u64,
            blocks: 1,
            atime: time::Timespec::new(0, 0),
            mtime: time::Timespec::new(0, 0),
            ctime: time::Timespec::new(0, 0),
            crtime: time::Timespec::new(0, 0),
            kind: FileType::RegularFile,
            perm: if self.read_only { 0o444 } else { 0o666 },
            nlink: 1,
            uid: 501,
            gid: 20,
            rdev: 0,
            flags: 0,
        }
    }
}

fn name_3ds_to_str(name: &[u8; 16]) -> String {
    let trimmed: Vec<u8> = name.iter().cloned().take_while(|c| *c != 0).collect();
    std::str::from_utf8(&trimmed).unwrap().to_owned()
}

fn name_os_to_3ds(name: &OsStr) -> [u8; 16] {
    // TODO better name conversion
    let mut name_converted = [0; 16];
    let utf8 = name.to_str().unwrap().as_bytes();
    let len = std::cmp::min(16, utf8.len());
    name_converted[0..len].copy_from_slice(&utf8[0..len]);
    name_converted
}

enum Ino {
    Dir(u32),
    File(u32),
}

impl Ino {
    fn to_os(&self) -> u64 {
        match *self {
            Ino::Dir(ino) => u64::from(ino),
            Ino::File(ino) => u64::from(ino) + 0x1_0000_0000,
        }
    }

    fn from_os(ino: u64) -> Ino {
        if ino > 0x1_0000_0000 {
            Ino::File((ino - 0x1_0000_0000) as u32)
        } else {
            Ino::Dir(ino as u32)
        }
    }
}

impl Drop for SaveDataFilesystem {
    fn drop(&mut self) {
        if !self.read_only {
            self.save.commit().unwrap();
            println!("Saved");
        }
    }
}

impl Filesystem for SaveDataFilesystem {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_converted = name_os_to_3ds(name);

        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = Dir::open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                if let Ok(child) = parent_dir.open_sub_dir(name_converted) {
                    let children_len = if let Ok(chidren) = child.list_sub_dir() {
                        chidren.len()
                    } else {
                        reply.error(EIO);
                        return;
                    };

                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &self.make_dir_attr(Ino::Dir(child.get_ino()).to_os(), children_len),
                        0,
                    );
                    return;
                }
                if let Ok(child) = parent_dir.open_sub_file(name_converted) {
                    reply.entry(
                        &time::Timespec::new(1, 0),
                        &self.make_file_attr(Ino::File(child.get_ino()).to_os(), child.len()),
                        0,
                    );
                    return;
                }
                reply.error(ENOENT);
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        match Ino::from_os(ino) {
            Ino::File(ino) => {
                if let Ok(file) = File::open_ino(self.save.clone(), ino) {
                    reply.attr(
                        &time::Timespec::new(1, 0),
                        &self.make_file_attr(Ino::File(file.get_ino()).to_os(), file.len()),
                    );
                } else {
                    reply.error(ENOENT);
                }
            }
            Ino::Dir(ino) => {
                if let Ok(dir) = Dir::open_ino(self.save.clone(), ino) {
                    let children_len = if let Ok(chidren) = dir.list_sub_dir() {
                        chidren.len()
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    reply.attr(
                        &time::Timespec::new(1, 0),
                        &self.make_dir_attr(Ino::Dir(dir.get_ino()).to_os(), children_len),
                    );
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn mkdir(&mut self, _req: &Request, parent: u64, name: &OsStr, _mode: u32, reply: ReplyEntry) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted = name_os_to_3ds(name);
        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = Dir::open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };
                match parent_dir.new_sub_dir(name_converted) {
                    Ok(child) => reply.entry(
                        &time::Timespec::new(1, 0),
                        &self.make_dir_attr(Ino::Dir(child.get_ino()).to_os(), 0),
                        0,
                    ),
                    Err(Error::AlreadyExist) => reply.error(EEXIST),
                    Err(Error::NoSpace) => reply.error(ENOSPC),
                    Err(_) => reply.error(EIO),
                }
                return;
            }
        }
    }

    fn mknod(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted = name_os_to_3ds(name);
        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = Dir::open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                match parent_dir.new_sub_file(name_converted, 0) {
                    Ok(child) => reply.entry(
                        &time::Timespec::new(1, 0),
                        &self.make_file_attr(Ino::File(child.get_ino()).to_os(), 0),
                        0,
                    ),
                    Err(Error::AlreadyExist) => reply.error(EEXIST),
                    Err(Error::NoSpace) => reply.error(ENOSPC),
                    Err(_) => reply.error(EIO),
                }
                return;
            }
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted = name_os_to_3ds(name);

        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = Dir::open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                if let Ok(child) = parent_dir.open_sub_dir(name_converted) {
                    match child.delete() {
                        Ok(None) => reply.ok(),
                        Ok(Some(_)) => reply.error(ENOTEMPTY),
                        Err(_) => reply.error(EIO),
                    }
                    return;
                }
                reply.error(ENOENT);
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }
        let name_converted = name_os_to_3ds(name);

        match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
            }
            Ino::Dir(ino) => {
                let parent_dir = if let Ok(parent_dir) = Dir::open_ino(self.save.clone(), ino) {
                    parent_dir
                } else {
                    reply.error(EIO);
                    return;
                };

                if let Ok(child) = parent_dir.open_sub_file(name_converted) {
                    match child.delete() {
                        Ok(()) => reply.ok(),
                        Err(_) => reply.error(EIO),
                    }
                    return;
                }
                reply.error(ENOENT);
            }
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: u32, reply: ReplyOpen) {
        match Ino::from_os(ino) {
            Ino::File(ino) => {
                if let Ok(file) = File::open_ino(self.save.clone(), ino) {
                    self.fh_map.insert(self.next_fh, file);
                    reply.opened(self.next_fh, 0);
                    self.next_fh += 1;
                } else {
                    reply.error(ENOENT);
                }
            }
            Ino::Dir(_) => {
                reply.error(EISDIR);
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: u32,
        _lock_owner: u64,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.fh_map.remove(&fh);
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        reply: ReplyData,
    ) {
        let offset = offset as usize;
        let size = size as usize;
        if let Some(file) = self.fh_map.get(&fh) {
            if size == 0 {
                reply.data(&[]);
                return;
            }
            let end = std::cmp::min(offset + size, file.len());
            if end <= offset {
                reply.data(&[]);
                return;
            }
            let mut buf = vec![0; end - offset];
            match file.read(offset, &mut buf) {
                Ok(()) | Err(Error::HashMismatch) => reply.data(&buf),
                _ => reply.error(EIO),
            }
        } else {
            reply.error(EBADF);
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _flags: u32,
        reply: ReplyWrite,
    ) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }

        let offset = offset as usize;
        let end = offset + data.len();
        if let Some(file) = self.fh_map.get_mut(&fh) {
            if data.is_empty() {
                reply.written(0);
                return;
            }
            if end > file.len() {
                match file.resize(end) {
                    Ok(()) => (),
                    Err(Error::NoSpace) => {
                        reply.error(ENOSPC);
                        return;
                    }
                    Err(_) => {
                        reply.error(EIO);
                        return;
                    }
                }
                match file.write(offset, &data) {
                    Ok(()) => reply.written(data.len() as u32),
                    _ => reply.error(EIO),
                }
            }
        } else {
            reply.error(EBADF);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        match Ino::from_os(ino) {
            Ino::File(_) => reply.error(ENOTDIR),
            Ino::Dir(ino) => {
                if let Ok(dir) = Dir::open_ino(self.save.clone(), ino) {
                    let parent_ino = if ino == 1 { 1 } else { dir.get_parent_ino() };
                    let mut entries = vec![
                        (Ino::Dir(ino).to_os(), FileType::Directory, ".".to_owned()),
                        (
                            Ino::Dir(parent_ino).to_os(),
                            FileType::Directory,
                            "..".to_owned(),
                        ),
                    ];

                    let sub_dirs = if let Ok(r) = dir.list_sub_dir() {
                        r
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    for (name, i) in sub_dirs {
                        entries.push((
                            Ino::Dir(i).to_os(),
                            FileType::Directory,
                            name_3ds_to_str(&name),
                        ));
                    }

                    let sub_files = if let Ok(r) = dir.list_sub_file() {
                        r
                    } else {
                        reply.error(EIO);
                        return;
                    };
                    for (name, i) in sub_files {
                        entries.push((
                            Ino::File(i).to_os(),
                            FileType::RegularFile,
                            name_3ds_to_str(&name),
                        ));
                    }

                    let to_skip = if offset == 0 { offset } else { offset + 1 } as usize;
                    for (i, entry) in entries.into_iter().enumerate().skip(to_skip) {
                        reply.add(entry.0, i as i64, entry.1, entry.2);
                    }
                    reply.ok();
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEmpty,
    ) {
        if self.read_only {
            reply.error(EROFS);
            return;
        }

        let name_converted = name_os_to_3ds(name);
        let newname_converted = name_os_to_3ds(newname);

        let dir = match Ino::from_os(parent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
                return;
            }
            Ino::Dir(ino) => match Dir::open_ino(self.save.clone(), ino) {
                Ok(dir) => dir,
                Err(_) => {
                    reply.error(EIO);
                    return;
                }
            },
        };

        let newdir = match Ino::from_os(newparent) {
            Ino::File(_) => {
                reply.error(ENOTDIR);
                return;
            }
            Ino::Dir(ino) => match Dir::open_ino(self.save.clone(), ino) {
                Ok(dir) => dir,
                Err(_) => {
                    reply.error(EIO);
                    return;
                }
            },
        };

        if let Ok(mut file) = dir.open_sub_file(name_converted) {
            if let Ok(old_file) = newdir.open_sub_file(newname_converted) {
                match old_file.delete() {
                    Ok(()) => (),
                    Err(_) => {
                        reply.error(EIO);
                        return;
                    }
                }
            }

            match file.rename(&newdir, newname_converted) {
                Ok(()) => reply.ok(),
                Err(Error::AlreadyExist) => reply.error(EEXIST),
                Err(_) => reply.error(EIO),
            }
        } else if let Ok(mut dir) = dir.open_sub_dir(name_converted) {
            if let Ok(old_dir) = newdir.open_sub_dir(newname_converted) {
                match old_dir.delete() {
                    Ok(None) => (),
                    Ok(Some(_)) => {
                        reply.error(ENOTEMPTY);
                        return;
                    }
                    Err(_) => {
                        reply.error(EIO);
                        return;
                    }
                }
            }

            match dir.rename(&newdir, newname_converted) {
                Ok(()) => reply.ok(),
                Err(Error::AlreadyExist) => reply.error(EEXIST),
                Err(_) => reply.error(EIO),
            }
        } else {
            reply.error(ENOENT);
        }
    }
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [OPTIONS] MOUNT_PATH", program);
    print!("{}", opts.usage(&brief));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("b", "boot9", "boot9.bin file path", "DIR");
    opts.optflag("h", "help", "print this help menu");
    opts.optopt("m", "movable", "movable.sed file path", "FILE");
    opts.optflag("r", "readonly", "mount as read-only file system");
    opts.optopt("", "bare", "mount a bare DISA file", "FILE");
    opts.optopt("", "sd", "SD root path", "DIR");
    opts.optopt("", "sdsave", "mount the SD save with the ID", "ID");
    opts.optopt("", "nand", "NAND root path", "DIR");
    opts.optopt("", "nandsave", "mount the NAND save with the ID", "ID");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            println!("Failed to parse the arguments: {}", f);
            print_usage(&program, opts);
            return;
        }
    };

    if matches.opt_present("h") {
        print_usage(&program, opts);
        return;
    }

    if matches.free.len() != 1 {
        println!("Please specify one mount path");
        return;
    }

    let boot9_path = matches.opt_str("boot9");
    let movable_path = matches.opt_str("movable");
    let bare_path = matches.opt_str("bare");
    let sd_path = matches.opt_str("sd");
    let sd_id = matches.opt_str("sdsave");
    let nand_path = matches.opt_str("nand");
    let nand_id = matches.opt_str("nandsave");

    if [&sd_id, &nand_id, &bare_path]
        .iter()
        .map(|x| if x.is_none() { 0 } else { 1 })
        .sum::<i32>()
        != 1
    {
        println!("One and only one of the following arguments must be supplied: --sdsave, --nandsave, --bare");
        return;
    }

    let resource = Resource::new(boot9_path, movable_path, sd_path, nand_path)
        .expect("Failed to load resource");

    let save = if let Some(bare) = bare_path {
        println!(
            "WARNING: After modification, you need to sign the CMAC header using other tools."
        );

        resource.open_bare_save(&bare).expect("Failed to open save")
    } else if let Some(id) = nand_id {
        let id = u32::from_str_radix(&id, 16).expect("Invalid ID");
        resource.open_nand_save(id).expect("Failed to open save")
    } else if let Some(id) = sd_id {
        let id = u64::from_str_radix(&id, 16).expect("Invalid ID");
        resource.open_sd_save(id).expect("Failed to open save")
    } else {
        panic!()
    };

    let fs = SaveDataFilesystem::new(save, matches.opt_present("r"));
    let options = [];
    let mountpoint = std::path::Path::new(&matches.free[0]);

    println!("Start mounting");
    mount(fs, &mountpoint, &options).unwrap();
}