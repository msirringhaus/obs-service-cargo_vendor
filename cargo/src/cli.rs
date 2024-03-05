// SPDX-License-Identifier: MPL-2.0

// Copyright (C) 2023  Soc Virnyl Estela

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::fmt::{self, Display};
use std::io;
use std::path::{Path, PathBuf};

use crate::consts::{
    BZ2_MIME, GZ_MIME, SUPPORTED_MIME_TYPES, VENDOR_PATH_PREFIX, XZ_MIME, ZST_MIME,
};
use crate::errors::OBSCargoError;
use crate::errors::OBSCargoErrorKind;
use crate::utils;

use clap::{Parser, ValueEnum};
use infer;

#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn, Level};

#[derive(Parser, Debug)]
#[command(
    author,
    name = "cargo_vendor",
    version,
    about = "OBS Source Service to vendor all crates.io and dependencies for Rust project locally",
    after_long_help = "Set verbosity and tracing through `RUST_LOG` environmental variable e.g. `RUST_LOG=trace`

Bugs can be reported on GitHub: https://github.com/openSUSE/obs-service-cargo_vendor/issues",
    max_term_width = 120
)]
pub struct Opts {
    #[clap(flatten)]
    pub src: Src,
    #[arg(
        long,
        value_enum,
        default_value_t,
        help = "What compression algorithm to use."
    )]
    pub compression: Compression,
    #[arg(
        long,
        help = "Tag some files for multi-vendor and multi-cargo_config projects"
    )]
    pub tag: Option<String>,
    #[arg(long, help = "Other cargo manifest files to sync with during vendor")]
    pub cargotoml: Vec<PathBuf>,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, help = "Update dependencies or not")]
    pub update: bool,
    #[arg(long, help = "Where to output vendor.tar* and cargo_config")]
    pub outdir: PathBuf,
    #[arg(
        long,
        default_value = "auto",
        default_missing_value = "always",
        value_name = "WHEN",
        help = "Whether WHEN to color output or not"
    )]
    pub color: clap::ColorChoice,

    #[arg(
        long,
        help = "A list of rustsec-id's to ignore. By setting this value, you acknowledge that this issue does not affect your package and you should be exempt from resolving it."
    )]
    pub i_accept_the_risk: Vec<String>,
    #[arg(
        long,
        help = "Patches that should be applied when vendoring (doing: vendor, apply patch, re-vendor)"
    )]
    pub patch: Vec<PathBuf>,
}

impl AsRef<Opts> for Opts {
    #[inline]
    fn as_ref(&self) -> &Opts {
        self
    }
}

#[derive(ValueEnum, Default, Debug, Clone, Copy)]
pub enum Compression {
    Gz,
    Xz,
    #[default]
    Zst,
    Bz2,
}

#[derive(Debug)]
pub enum SupportedFormat {
    Compressed(Compression, PathBuf),
    Dir(PathBuf),
}

impl Display for SupportedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            SupportedFormat::Compressed(comp_type, src) => {
                format!("Compression: {}, Src: {}", comp_type, src.display())
            }
            SupportedFormat::Dir(src) => format!("Directory: {}", src.display()),
        };
        write!(f, "{}", msg)
    }
}

#[derive(Debug)]
pub struct UnsupportedFormat {
    pub ext: String,
}

impl Display for UnsupportedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unsupported archive format {}", self.ext)
    }
}

impl std::error::Error for UnsupportedFormat {}

impl Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let msg = match self {
            Compression::Gz => "gz",
            Compression::Xz => "xz",
            Compression::Zst => "zst",
            Compression::Bz2 => "bz2",
        };
        write!(f, "{}", msg)
    }
}

#[derive(clap::Args, Debug, Clone)]
pub struct Src {
    #[arg(
        long,
        visible_aliases = ["srctar", "srcdir"],
        help = "Where to find sources. Source is either a directory or a source tarball AND cannot be both."
    )]
    pub src: PathBuf,
}

impl Src {
    pub fn new(p: &Path) -> Self {
        Self { src: p.into() }
    }
}

pub trait Vendor {
    fn is_supported(&self) -> Result<SupportedFormat, UnsupportedFormat>;
    fn run_vendor(&self, opts: &Opts) -> Result<(), OBSCargoError>;
}

pub fn decompress(comp_type: &Compression, outdir: &Path, src: &Path) -> io::Result<()> {
    match comp_type {
        Compression::Gz => utils::decompress::targz(outdir, src),
        Compression::Xz => utils::decompress::tarxz(outdir, src),
        Compression::Zst => utils::decompress::tarzst(outdir, src),
        Compression::Bz2 => utils::decompress::tarbz2(outdir, src),
    }
}

impl Vendor for Src {
    fn is_supported(&self) -> Result<SupportedFormat, UnsupportedFormat> {
        if let Ok(actual_src) = utils::process_globs(&self.src) {
            debug!(?actual_src, "Source got from glob pattern");
            if actual_src.is_file() {
                match infer::get_from_path(&actual_src) {
                    Ok(kind) => match kind {
                        Some(known) => {
                            if SUPPORTED_MIME_TYPES.contains(&known.mime_type()) {
                                trace!(?known);
                                if known.mime_type().eq(GZ_MIME) {
                                    Ok(SupportedFormat::Compressed(Compression::Gz, actual_src))
                                } else if known.mime_type().eq(XZ_MIME) {
                                    Ok(SupportedFormat::Compressed(Compression::Xz, actual_src))
                                } else if known.mime_type().eq(ZST_MIME) {
                                    Ok(SupportedFormat::Compressed(Compression::Zst, actual_src))
                                } else if known.mime_type().eq(BZ2_MIME) {
                                    Ok(SupportedFormat::Compressed(Compression::Bz2, actual_src))
                                } else {
                                    unreachable!()
                                }
                            } else {
                                Err(UnsupportedFormat {
                                    ext: known.mime_type().to_string(),
                                })
                            }
                        }
                        None => Err(UnsupportedFormat {
                            ext: "`File type is not known`".to_string(),
                        }),
                    },
                    Err(err) => {
                        error!(?err);
                        Err(UnsupportedFormat {
                            ext: "`Cannot read file`".to_string(),
                        })
                    }
                }
            } else {
                Ok(SupportedFormat::Dir(actual_src))
            }
        } else {
            error!("Sources cannot be determined!");
            Err(UnsupportedFormat {
                ext: format!("unsupported source {}", &self.src.display()),
            })
        }
    }

    fn run_vendor(&self, opts: &Opts) -> Result<(), OBSCargoError> {
        let tmpdir = match tempfile::Builder::new()
            .prefix(VENDOR_PATH_PREFIX)
            .rand_bytes(8)
            .tempdir()
        {
            Ok(t) => t,
            Err(err) => {
                error!("{}", err);
                return Err(OBSCargoError::new(
                    OBSCargoErrorKind::VendorError,
                    "failed to create temporary directory for vendor process".to_string(),
                ));
            }
        };

        let workdir: PathBuf = tmpdir.path().into();
        debug!(?workdir, "Created working directory");

        // Return workdir here?
        let newworkdir: PathBuf = match self.is_supported() {
            Ok(format) => {
                let dir = match format {
                    SupportedFormat::Compressed(compression_type, ref srcpath) => {
                        match decompress(&compression_type, &workdir, srcpath) {
                            Ok(_) => {
                                let dirs: Vec<Result<std::fs::DirEntry, std::io::Error>> =
                                    std::fs::read_dir(&workdir)
                                        .map_err(|err| {
                                            error!(?err, "Failed to read directory");
                                            OBSCargoError::new(
                                                OBSCargoErrorKind::VendorError,
                                                "failed to read directory".to_string(),
                                            )
                                        })?
                                        .collect();
                                trace!(?dirs, "List of files and directories of the workdir");
                                // If length is one, this means that the project has
                                // a top-level folder
                                if dirs.len() != 1 {
                                    debug!(?workdir);
                                    workdir
                                } else {
                                    match dirs.into_iter().last() {
                                        Some(p) => match p {
                                            Ok(dir) => {
                                                if dir.path().is_dir() {
                                                    debug!("{}", dir.path().display());
                                                    dir.path()
                                                } else {
                                                    error!(?dir, "Tarball was extracted but got a file and not a possible top-level directory.");
                                                    return Err(OBSCargoError::new(OBSCargoErrorKind::VendorError, "No top-level directory found after tarball was extracted".to_string()));
                                                }
                                            }
                                            Err(err) => {
                                                error!(?err, "Failed to read directory entry");
                                                return Err(OBSCargoError::new(
                                                    OBSCargoErrorKind::VendorError,
                                                    err.to_string(),
                                                ));
                                            }
                                        },
                                        None => {
                                            error!("This should be unreachable here");
                                            unreachable!();
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                return Err(OBSCargoError::new(
                                    OBSCargoErrorKind::VendorError,
                                    err.to_string(),
                                ));
                            }
                        }
                    }
                    SupportedFormat::Dir(ref srcpath) => match utils::copy_dir_all(
                        srcpath,
                        &workdir.join(srcpath.file_name().unwrap_or(srcpath.as_os_str())),
                    ) {
                        Ok(_) => workdir.join(srcpath.file_name().unwrap_or(srcpath.as_os_str())),
                        Err(err) => {
                            return Err(OBSCargoError::new(
                                OBSCargoErrorKind::VendorError,
                                err.to_string(),
                            ))
                        }
                    },
                };

                // Copying patches to the new temporary working-dir to be able to apply them later
                for patch in &opts.patch {
                    match format {
                        SupportedFormat::Compressed(_, ref srcpath)
                        | SupportedFormat::Dir(ref srcpath) => {
                            if let Some(dirname) = srcpath.parent() {
                                std::fs::copy(dirname.join(patch), dir.join(patch)).map_err(
                                    |err| {
                                        error!(?err, "Failed to copy patch");
                                        OBSCargoError::new(
                                            OBSCargoErrorKind::PatchError,
                                            "failed to copy patch".to_string(),
                                        )
                                    },
                                )?;
                            }
                        }
                    }
                }
                dir
            }
            Err(err) => {
                error!(?err);
                return Err(OBSCargoError::new(
                    OBSCargoErrorKind::VendorError,
                    err.to_string(),
                ));
            }
        };

        debug!(?newworkdir, "Workdir updated!");

        match utils::process_src(opts, &newworkdir) {
            Ok(_) => {
                info!("🥳 ✨ Successfull ran OBS Service Cargo Vendor ✨");
            }
            Err(err) => {
                error!(?err);
                return Err(OBSCargoError::new(
                    OBSCargoErrorKind::VendorError,
                    err.to_string(),
                ));
            }
        };
        drop(newworkdir);
        tmpdir
            .close()
            .map_err(|err| OBSCargoError::new(OBSCargoErrorKind::VendorError, err.to_string()))
    }
}
