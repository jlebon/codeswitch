/* Copyright (C) 2018 Jonathan Lebon <jonathan@jlebon.com>
 * SPDX-License-Identifier: MIT
 * */

use std::io;
use std::fs;
use std::str::FromStr;
use std::path::Path;
use std::path::PathBuf;
use std::io::Write;
use std::io::BufRead;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;

use std::collections::HashSet;
use std::collections::HashMap;

#[macro_use]
extern crate clap;
extern crate ansi_term;
extern crate byteorder;
extern crate openat;

use ansi_term::Colour::Red;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use openat::{Dir, SimpleType};

/* let's be academic and properly handle invalid Unicode filepaths, which
 * basically entails using OsString instead of String */

fn main() {

    let matches = clap_app!((crate_name!()) =>
            (version: crate_version!())
            (author: crate_authors!())
            (about: crate_description!())
            (@arg DIR: +required "The root directory to search")
            (@arg CODEBASE: +required "The codebase to search for (or '_' for all)")
            (@arg FILTER: "String to filter by, or line index to return")
            (@arg rebuild: -f --rebuild "Force rebuild of cache")
        ).get_matches();

    let dirpath: &Path = Path::new(matches.value_of_os("DIR").unwrap());
    let codebase: &OsStr = matches.value_of_os("CODEBASE").unwrap();
    let filter: &OsStr = matches.value_of_os("FILTER").unwrap_or_else(|| OsStr::new(""));

    if let Err(e) = run(dirpath, codebase, filter, matches.is_present("rebuild")) {
        let _ = writeln!(std::io::stderr(), "{} {}", Red.bold().paint("error:"), e);
        std::process::exit(1);
    }
}

fn run(
    dirpath:         &Path,
    wanted_codebase: &OsStr,
    filter:          &OsStr,
    force_rebuild:   bool
) -> io::Result<()> {

    let dir = Dir::open(dirpath)?;

    let meta = dir.metadata(".")?;
    if !meta.is_dir() {
        return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{:?} is not a directory", dir)));
    }

    let cachedir = match std::env::home_dir() {
        Some(path) => path.join(".cache"),
        None => PathBuf::from("/var/cache"),
    };

    if !cachedir.is_dir() {
        return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Cache directory {:?} not found", cachedir)));
    }

    let mut was_cached = false;
    let cachefn = cachedir.join(crate_name!());
    let mut codebases =
        if force_rebuild {
            build_cache(&dir, &cachefn)?
        } else {
            match read_cache(&dir, &cachefn)? {
                Option::None => build_cache(&dir, &cachefn)?,
                Option::Some(codebases) => {
                    was_cached = true;
                    codebases
                },
            }
        };

    /* short-circuit for '_' support, e.g. for shell auto-completion */
    if wanted_codebase == "_" {
        /* add to set to make unique */
        let mut basenames: HashSet<&OsStr> = HashSet::new();
        for codebase in &codebases {
            /* we can safely unwrap here, our paths are all well-formed */
            basenames.insert(codebase.file_name().unwrap());
        }
        for basename in &basenames {
            io::stdout().write_all(basename.as_bytes())?;
            println!();
        }
        return Ok(());
    }

    codebases.retain(|path| path.ends_with(wanted_codebase));

    /* if we didn't find anything but the cache isn't fresh, let's try rescanning */
    if codebases.is_empty() && was_cached {
        codebases = build_cache(&dir, &cachefn)?;
        codebases.retain(|path| path.ends_with(wanted_codebase));
    }

    /* are we filtering by number? */
    if let Ok(idx) = usize::from_str(&filter.to_string_lossy()) {
        if !(0 < idx && idx <= codebases.len()) {
            print_codebases(dirpath, &codebases)?;
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                      format!("Index {} out of range", idx)));
        }
        print_codebase(dirpath, &codebases[idx-1])?;
    } else {

        /* are we filtering by string? */
        if !filter.is_empty() {
            codebases.retain(|path| {
                /* we don't want to match the codebase itself again; just its dirpath */
                let dirpath_len = path.as_os_str().len() - wanted_codebase.len();
                let dirpath = &path.as_os_str().as_bytes()[..(dirpath_len)];
                /* creative substring search for &[u8]:
                 * https://stackoverflow.com/a/35907071/308136 */
                let mut windows = dirpath.windows(filter.len());
                windows.find(|&window| window == filter.as_bytes()) != None
            });
        }

        match codebases.len() {
            0 => return Err(io::Error::new(io::ErrorKind::NotFound,
                                           "No matches found")),
            1 => (),
            _ => {
                print_codebases(dirpath, &codebases)?;
                return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                          "Multiple matches found"))
            },
        }

        print_codebase(dirpath, &codebases[0])?;
    }

    Ok(())
}

fn print_codebase(dir: &Path, codebase: &Path) -> io::Result<()> {
    io::stdout().write_all(dir.join(codebase).as_os_str().as_bytes())?;
    println!();
    Ok(())
}

fn print_codebases(dir: &Path, codebases: &[PathBuf]) -> io::Result<()> {
    for (i, codebase) in codebases.iter().enumerate() {
        print!("  {:2}  ", i+1);
        print_codebase(dir, codebase)?;
    }
    Ok(())
}

fn read_cache(cached_dir: &Dir, cache: &Path) -> io::Result<Option<Vec<PathBuf>>> {
    match fs::File::open(cache) {
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                Err(e)
            } else {
                Ok(None)
            }
        },
        Ok(f) => Ok(read_cache_file(cached_dir, &f)?),
    }
}

fn read_cache_file(cached_dir: &Dir, file: &fs::File) -> io::Result<Option<Vec<PathBuf>>> {

    let meta = cached_dir.metadata(".")?;
    let stat = meta.stat();

    let mut reader = io::BufReader::new(file);

    /* first read dev and inode and check that they match */
    let cached_dev = reader.read_u64::<LittleEndian>()?;
    let cached_ino = reader.read_u64::<LittleEndian>()?;

    if cached_dev != stat.st_dev || cached_ino != stat.st_ino {
        return Ok(None);
    }

    let mut codebases = Vec::new();
    loop {
        let mut buf = Vec::new();
        let n = reader.read_until(b'\0', &mut buf)?;
        if n == 0 {
            if codebases.is_empty() {
                return Ok(None);
            }
            return Ok(Some(codebases));
        }

        /* trim tail */
        while !buf.is_empty() && buf[buf.len()-1] == b'\0' {
            buf.pop();;
        }

        codebases.push(PathBuf::from(OsString::from_vec(buf)));
    }
}

fn build_cache(cached_dir: &Dir, cache: &Path) -> io::Result<Vec<PathBuf>> {

    /* first, scan the target dir */
    let codebases = scan_dir(&cached_dir)?;

    /* ok, let's write it to cache */

    let file = fs::File::create(cache)?;
    let mut writer = io::BufWriter::new(file);

    /* store cached dir inode first so it works regardless of different paths due to
     * symlinks/bind-mounts (e.g. in my pet container, I use /code, outside ~/Code) */
    let meta = cached_dir.metadata(".")?;
    let stat = meta.stat();
    writer.write_u64::<LittleEndian>(stat.st_dev)?;
    writer.write_u64::<LittleEndian>(stat.st_ino)?;

    for codebase in &codebases {
        writer.write_all(codebase.as_os_str().as_bytes())?;
        writer.write_all(b"\0")?;
    }

    Ok(codebases)
}

fn scan_dir(dir: &Dir) -> io::Result<Vec<PathBuf>> {
    let mut codebases = Vec::new();
    /* Note here that the pathbuf stack we init is *not* initialized with a dirpath. The
     * cache then purely holds paths relative to dir. */
    scan_dir_recurse(dir, &mut PathBuf::new(), &mut codebases)?;
    Ok(codebases)
}

#[derive(PartialEq)]
enum DirType {
    Leaf,   /* i.e. the dir is a codebase itself */
    Branch,
}

fn scan_dir_recurse(
    dir:       &Dir,
    path:      &mut PathBuf,
    codebases: &mut Vec<PathBuf>
) -> io::Result<DirType> {

    /* We want to return a list of subpaths which have a .git dir with symlinks substituted
     * into middle components if they're shorter. Leaf dirs (codebases) are always added
     * once using its real subdir and once using its symlink if exists */

    match dir.metadata(".git") {
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                return Err(e);
            }
        },
        Ok(meta) => {
            /* only add to list if it's a dir. otherwise (submodule?) let's skip */
            if meta.is_dir() {
                codebases.push(path.clone());
                return Ok(DirType::Leaf);
            }
            return Ok(DirType::Branch);
        },
    };

    /* no .git/ dir, let's recurse */

    /* collect symlinks and subdirs */
    let mut subdirs: HashSet<OsString> = HashSet::new();
    let mut symlinks: HashMap<OsString, OsString> = HashMap::new();
    for entry in dir.list_dir(".")? {
        let entry = entry?;
        let ftype = match entry.simple_type() {
            Some(ftype) => ftype,
            /* stat() fallback */
            None => dir.metadata(entry.file_name())?.simple_type(),
        };
        if ftype == SimpleType::Dir {
            subdirs.insert(entry.file_name().to_os_string());
        } else if ftype == SimpleType::Symlink {
            let link = entry.file_name().to_os_string();
            let target = dir.read_link(entry.file_name())?.into_os_string();
            /* but only if it's actually shorter */
            if link.len() < target.len() {
                symlinks.insert(link, target);
            }
        }
    }

    /* prune away dead symlinks */
    symlinks.retain(|_, target| subdirs.contains(target));

    /* prune away subdirs for which we have symlinks that target them */
    for target in symlinks.values() {
        subdirs.remove(target);
    }

    /* recurse into symlinks */
    for (symlink, target) in &symlinks {
        path.push(symlink);
        let dtype = scan_dir_recurse(&dir.sub_dir(target.as_os_str())?, path, codebases)?;
        path.pop();
        /* make sure we also add the non-symlink version if it was a codebase */
        if dtype == DirType::Leaf {
            codebases.push(path.join(target));
        }
    }

    /* recurse into the other subdirs */
    for subdir in &subdirs {
        path.push(subdir);
        scan_dir_recurse(&dir.sub_dir(subdir.as_os_str())?, path, codebases)?;
        path.pop();
    }

    Ok(DirType::Branch)
}
