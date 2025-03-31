// lvm2.rs
//#![no_std]
extern crate alloc;

use acid_io::{Read, Seek, SeekFrom};
use alloc::string::{String, ToString};
use header::PhysicalVolumeHeader;
use log::debug;
use serde::Deserialize;
use snafu::{ensure, OptionExt, ResultExt, Snafu};

use crate::header::{MetadataAreaHeader, PhysicalVolumeLabelHeader};
use crate::metadata::{deserialize::MetadataElements, MetadataRoot};

// Vocabulary: in this crate we use the term "sheet" to describe a block of exactly 512 bytes
// (to avoid confusion around the word "sector")
// // spec: https://github.com/libyal/libvslvm/blob/ab09a380072448d9c84c886d487d8c3dfa2d1527/documentation/Logical%20Volume%20Manager%20(LVM)%20format.asciidoc#2-physical-volume-label

pub struct Lvm2 {
    pvh: PhysicalVolumeHeader,
    pv_name: String,
    vg_name: String,
    vg_config: MetadataRoot,
}

#[derive(Debug, Snafu)]
pub enum Error {
    Io {
        #[cfg(not(feature = "std"))]
        #[snafu(source(from(acid_io::Error, no_std::AcidIoError)))]
        source: no_std::AcidIoError,
        #[cfg(feature = "std")]
        source: acid_io::Error,
    },
    WrongMagic,
    ParseError {
        error: String,
    },
    MultipleVGsError,
    PVDoesntContainItself,
    Serde {
        #[cfg(not(feature = "std"))]
        #[snafu(source(from(serde::de::value::Error, no_std::SerdeDeError)))]
        source: no_std::SerdeDeError,
        #[cfg(feature = "std")]
        source: serde::de::value::Error,
    },
    MissingMetadata,
}

#[cfg(not(feature = "std"))]
mod no_std {
    pub struct AcidIoError(pub acid_io::Error);
    impl snafu::Error for AcidIoError {}
    impl ::core::fmt::Display for AcidIoError {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Display::fmt(&self.0, f)
        }
    }
    impl ::core::fmt::Debug for AcidIoError {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Debug::fmt(&self.0, f)
        }
    }

    pub struct SerdeDeError(pub serde::de::value::Error);
    impl snafu::Error for SerdeDeError {}
    impl ::core::fmt::Display for SerdeDeError {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Display::fmt(&self.0, f)
        }
    }
    impl ::core::fmt::Debug for SerdeDeError {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Debug::fmt(&self.0, f)
        }
    }
}

mod force_de_typed_map;
mod header;
mod lv;
pub mod metadata;
pub use lv::*;

impl Lvm2 {
    // Public getters to expose pv_name and lvs for external use.
    pub fn pv_name(&self) -> &str {
        &self.pv_name
    }

    pub fn lvs(&self) -> impl Iterator<Item = LV> + '_ {
        self.vg_config
            .logical_volumes
            .iter()
            .map(|(name, desc)| LV { name, desc })
    }

    // Modified to take a mutable reference for the reader.
    pub fn open<T: Read + Seek>(reader: &mut T) -> Result<Self, Error> {
        reader.seek(SeekFrom::Start(512)).context(IoSnafu)?; // skip zero sheet

        let mut buf = [0u8; 512];
        reader.read_exact(&mut buf).context(IoSnafu)?; // read header
        tracing::trace!(?buf);

        let (_, vhl) = PhysicalVolumeLabelHeader::parse(&buf).map_err(|e| Error::ParseError {
            error: e.to_string(),
        })?;
        debug!(
            "PhysicalVolumeLabelHeader: sector_number: {}, checksum: {}, data_offset: {}",
            vhl.sector_number, vhl.checksum, vhl.data_offset
        );
        let (_, pvh) =
            PhysicalVolumeHeader::parse(&buf[(vhl.data_offset as usize)..]).map_err(|e| {
                Error::ParseError {
                    error: e.to_string(),
                }
            })?;
        tracing::trace!(?pvh);

        debug!(
            "PhysicalVolumeHeader: pv_ident: {}, pv_size: {}",
            pvh.pv_ident, pvh.pv_size
        );

        let metadata_descriptor = pvh
            .metadata_descriptors
            .first()
            .context(MissingMetadataSnafu)?;

        reader
            .seek(acid_io::SeekFrom::Start(metadata_descriptor.offset))
            .context(IoSnafu)?; // skip zero sheet
        reader.read_exact(&mut buf).context(IoSnafu)?;
        let (_, mah) = MetadataAreaHeader::parse(&buf).map_err(|e| Error::ParseError {
            error: e.to_string(),
        })?;
        tracing::trace!(?mah);
        debug!(
            "MetadataAreaHeader: checksum: {}, version: {}, metadata_area_offset: {}, metadata_area_size: {}",
            mah.checksum, mah.version, mah.metadata_area_offset, mah.metadata_area_size
        );

        let mut metadata = String::new();
        for locdesc in &mah.location_descriptors {
            reader
                .seek(acid_io::SeekFrom::Start(
                    metadata_descriptor.offset + locdesc.data_area_offset,
                ))
                .context(IoSnafu)?; // skip zero sheet
            reader
                .by_ref()
                .take(locdesc.data_area_size)
                .read_to_string(&mut metadata)
                .context(IoSnafu)?;
        }
        tracing::debug!(%metadata);

        let (trailing_garbage, metadata) =
            MetadataElements::parse(&metadata).map_err(|e| Error::ParseError {
                error: e.to_string(),
            })?;
        tracing::debug!(?trailing_garbage, ?metadata);

        let meta_root =
            force_de_typed_map::ForceDeTypedMap::<String, MetadataRoot>::deserialize(&metadata)
                .context(SerdeSnafu)?;
        tracing::debug!(?meta_root);

        ensure!(meta_root.0.len() == 1, MultipleVGsSnafu);
        let (vg_name, vg_config) = meta_root.0.into_iter().next().unwrap();

        let pv_name = vg_config
            .physical_volumes
            .iter()
            .find(|(_, v)| v.id.replace('-', "") == pvh.pv_ident)
            .context(PVDoesntContainItselfSnafu)?
            .0
            .clone();

        Ok(Self {
            pvh,
            pv_name,
            vg_name,
            vg_config,
        })
    }

    // Modified LV open functions: they now take a mutable reference for the reader.
    pub fn open_lv_by_name<'a, 'r, T: Read + Seek>(
        &'a self,
        name: &str,
        reader: &'r mut T,
    ) -> Option<OpenLV<'a, 'r, T>> {
        self.vg_config
            .logical_volumes
            .get_key_value(name)
            .map(move |(name, desc)| self.open_lv(LV { name, desc }, reader))
    }
    pub fn open_lv_by_id<'a, 'r, T: Read + Seek>(
        &'a self,
        id: &str,
        reader: &'r mut T,
    ) -> Option<OpenLV<'a, 'r, T>> {
        self.lvs()
            .find(|lv| lv.id() == id)
            .map(move |lv| self.open_lv(lv, reader))
    }
    pub fn open_lv<'a, 'r, T: Read + Seek>(
        &'a self,
        lv: LV<'a>,
        reader: &'r mut T,
    ) -> OpenLV<'a, 'r, T> {
        OpenLV {
            lv,
            lvm: self,
            reader,
            position: 0,
            current_segment_end: 0,
        }
    }

    pub fn pv_id(&self) -> &str {
        &self.vg_config.physical_volumes[&self.pv_name].id
    }

    pub fn vg_name(&self) -> &str {
        &self.vg_name
    }
    pub fn vg_id(&self) -> &str {
        &self.vg_config.id
    }

    pub fn extent_size(&self) -> u64 {
        self.vg_config.extent_size * 512
    }
}
