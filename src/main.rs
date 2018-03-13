/* Copyright (C) 2018 Jonathan Lebon <jonathan.lebon@gmail.com>
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

use ansi_term::Colour::Red;

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

    let dir: &Path = Path::new(matches.value_of_os("DIR").unwrap());
    let codebase: &OsStr = matches.value_of_os("CODEBASE").unwrap();
    let filter: &OsStr = matches.value_of_os("FILTER").unwrap_or(OsStr::new(""));

    if let Err(e) = main_impl(dir, codebase, filter, matches.is_present("rebuild")) {
        writeln!(std::io::stderr(), "{} {}", Red.bold().paint("error:"), e).unwrap();
        std::process::exit(1);
    }
}

fn main_impl(dir: &Path, wanted_codebase: &OsStr,
             filter: &OsStr, force_rebuild: bool) -> io::Result<()> {

    let meta = fs::metadata(dir)?;
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
    let cachefn = cachedir.join((crate_name!()));
    let mut codebases =
        if force_rebuild {
            build_cache(dir, &cachefn)?
        } else {
            match read_cache(dir, &cachefn)? {
                Option::None => build_cache(dir, &cachefn)?,
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
            io::stdout().write(basename.as_bytes())?;
            println!();
        }
        return Ok(());
    }

    codebases.retain(|path| path.ends_with(wanted_codebase));

    /* if we didn't find anything but the cache isn't fresh, let's try rescanning */
    if codebases.is_empty() && was_cached {
        codebases = build_cache(dir, &cachefn)?;
        codebases.retain(|path| path.ends_with(wanted_codebase));
    }

    /* are we filtering by number? */
    if let Ok(idx) = usize::from_str(&filter.to_string_lossy()) {
        if !(0 < idx && idx <= codebases.len()) {
            print_codebases(&codebases)?;
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                      format!("Index {} out of range", idx)));
        }
        io::stdout().write(codebases[idx-1].as_os_str().as_bytes())?;
    } else {

        /* are we filtering by string? */
        if filter.len() > 0 {
            codebases.retain(|path| {
                /* creative substring search for &[u8]:
                 * https://stackoverflow.com/a/35907071/308136 */
                let mut windows = path.as_os_str().as_bytes().windows(filter.len());
                windows.find(|&window| window == filter.as_bytes()) != None
            });
        }

        match codebases.len() {
            0 => return Err(io::Error::new(io::ErrorKind::NotFound,
                                           "no matches found")),
            1 => (),
            _ => {
                print_codebases(&codebases)?;
                return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                          "multiple matches found"))
            },
        }

        io::stdout().write(codebases[0].as_os_str().as_bytes())?;
    }
    println!();

    Ok(())
}

fn print_codebases(codebases: &Vec<PathBuf>) -> io::Result<()> {
    for (i, codebase) in codebases.iter().enumerate() {
        print!("  {:2}  ", i+1);
        io::stdout().write(codebase.as_os_str().as_bytes())?;
        println!();
    }
    Ok(())
}

fn read_cache(cached_dir: &Path, cache: &Path) -> io::Result<Option<Vec<PathBuf>>> {
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

fn read_cache_file(cached_dir: &Path, file: &fs::File) -> io::Result<Option<Vec<PathBuf>>> {
    let mut codebases = Vec::new();

    let mut first = true;
    let mut reader = io::BufReader::new(file);
    loop {
        let mut buf = Vec::new();
        let n = reader.read_until(b'\0', &mut buf)?;
        if n == 0 {
            if codebases.len() == 0 {
                return Ok(None);
            }
            return Ok(Some(codebases));
        }

        /* trim tail */
        while buf.len() > 0 && buf[buf.len()-1] == b'\0' {
            buf.pop();;
        }

        let codebase = PathBuf::from(OsString::from_vec(buf));

        /* we store the cached dir itself as the first entry; check it here */
        if first {
            if codebase != cached_dir {
                return Ok(None);
            }
        } else {
            codebases.push(codebase);
        }

        first = false;
    }
}

fn build_cache(cached_dir: &Path, cache: &Path) -> io::Result<Vec<PathBuf>> {

    /* first, scan the target dir */
    let codebases = scan_dir(&cached_dir)?;

    /* ok, let's write it to cache */

    let file = fs::File::create(cache)?;
    let mut writer = io::BufWriter::new(file);

    /* store cached dir itself as first entry */
    writer.write(cached_dir.as_os_str().as_bytes())?;
    writer.write(b"\0")?;

    for codebase in &codebases {
        writer.write(codebase.as_os_str().as_bytes())?;
        writer.write(b"\0")?;
    }

    Ok(codebases)
}

fn scan_dir(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut codebases = Vec::new();
    scan_dir_recurse(dir, &mut codebases)?;
    Ok(codebases)
}

#[derive(PartialEq)]
enum DirType {
    Leaf,   /* i.e. the dir is a codebase itself */
    Branch,
}

fn scan_dir_recurse(dir: &Path, codebases: &mut Vec<PathBuf>) -> io::Result<DirType> {

    /* We want to return a list of subpaths which have a .git dir with symlinks substituted
     * into middle components if they're shorter. Leaf dirs (codebases) are always added
     * once using its real subdir and once using its symlink if exists */

    match fs::symlink_metadata(dir.join(".git")) {
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                return Err(e);
            }
        },
        Ok(meta) => {
            /* only add to list if it's a dir. otherwise (submodule?) let's skip */
            if meta.is_dir() {
                codebases.push(dir.to_path_buf());
                return Ok(DirType::Leaf);
            }
            return Ok(DirType::Branch);
        },
    };

    /* no .git/ dir, let's recurse */

    /* collect symlinks and subdirs */
    let mut subdirs: HashSet<OsString> = HashSet::new();
    let mut symlinks: HashMap<OsString, OsString> = HashMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ftype = entry.file_type()?;
        if ftype.is_dir() {
            subdirs.insert(entry.file_name());
        } else if ftype.is_symlink() {
            let link = entry.file_name();
            let target = fs::read_link(entry.path())?.into_os_string();
            /* but only if it's actually shorter */
            if link.len() < target.len() {
                symlinks.insert(link, target);
            }
        }
    }

    /* prune away dead symlinks */
    symlinks.retain(|_, target| subdirs.contains(target));

    /* prune away subdirs for which we have symlinks that target them */
    for (_, target) in &symlinks {
        subdirs.remove(target);
    }

    /* recurse into symlinks */
    for (symlink, target) in &symlinks {
        let dtype = scan_dir_recurse(&dir.join(symlink), codebases)?;
        /* make sure we also add the non-symlink version if it was a codebase */
        if dtype == DirType::Leaf {
            codebases.push(dir.join(target));
        }
    }

    /* recurse into the other subdirs */
    for subdir in &subdirs {
        scan_dir_recurse(&dir.join(subdir), codebases)?;
    }

    Ok(DirType::Branch)
}
