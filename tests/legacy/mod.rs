//! The four `autobee` external types.
//!
//! They are supplied by the consumer, exactly like `encoding/legacy.js` is
//! supplied by autobee. The reference only decodes legacy records ("version 0
//! records cannot be encoded"), so encoding panics here too. Fixtures never
//! cover them: `preencode` throws in JS, so no reference bytes exist.

use bytes::Bytes;
use compact_encoding::{Codec, Reader, Result, Writer, Writer as W};

use crate::autobee_spec::autobase_compat::{Checkout, Clock, Node};

fn decode_only() -> ! {
    panic!("legacy records cannot be encoded")
}

#[derive(Debug, Clone, PartialEq)]
pub struct BootRecordV0 {
    pub key: [u8; 32],
    pub system_length: u64,
    pub heads: Clock,
}

impl Codec for BootRecordV0 {
    fn encoded_len(&self) -> usize {
        decode_only()
    }

    fn encode(&self, _: &mut Writer) {
        decode_only()
    }

    fn decode(r: &mut Reader) -> Result<Self> {
        let indexed = Checkout::decode(r)?;
        let heads = Clock::decode(r)?;
        Ok(Self {
            key: indexed.key,
            system_length: indexed.length,
            heads,
        })
    }
}

/// The reference rewinds the version byte, so a decoder needs the flags word
/// that the versioned wrapper already consumed.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemWriterV0 {
    pub flags: u64,
    pub length: u64,
}

impl Codec for SystemWriterV0 {
    fn encoded_len(&self) -> usize {
        decode_only()
    }

    fn encode(&self, _: &mut Writer) {
        decode_only()
    }

    fn decode(_: &mut Reader) -> Result<Self> {
        // `legacy.js` does `state.start--` here to re-read the version byte as
        // the flags word; Reader has no rewind, so versions 0-3 cannot be
        // decoded through the generated wrapper.
        Err(compact_encoding::Error::Invalid(
            "legacy system writers need a one byte rewind",
            0,
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexCheckpoint {
    signature: [u8; 64],
    length: u64,
}

impl Codec for IndexCheckpoint {
    fn encoded_len(&self) -> usize {
        decode_only()
    }

    fn encode(&self, _: &mut Writer) {
        decode_only()
    }

    fn decode(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            signature: r.fixed::<64>()?,
            length: r.uint()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checkpointer {
    checkpointer: u64,
    checkpoint: Option<IndexCheckpoint>,
}

impl Codec for Checkpointer {
    fn encoded_len(&self) -> usize {
        decode_only()
    }

    fn encode(&self, _: &mut Writer) {
        decode_only()
    }

    fn decode(r: &mut Reader) -> Result<Self> {
        let checkpointer = r.uint()?;
        let checkpoint = if checkpointer == 0 {
            Some(IndexCheckpoint::decode(r)?)
        } else {
            None
        };
        Ok(Self {
            checkpointer,
            checkpoint,
        })
    }
}

fn decode_array<T: Codec>(r: &mut Reader) -> Result<Vec<T>> {
    let n = r.uint()?;
    let mut out = Vec::with_capacity((n as usize).min(r.remaining()));
    for _ in 0..n {
        out.push(T::decode(r)?);
    }
    Ok(out)
}

fn decode_additional_data(r: &mut Reader) -> Result<()> {
    let flags = r.uint()?;
    if flags & 1 != 0 {
        let _ = r.fixed::<32>()?;
    }
    if flags & 2 != 0 {
        let _ = r.uint()?;
    }
    Ok(())
}

fn decode_additional(r: &mut Reader) -> Result<()> {
    let pointer = r.uint()?;
    if pointer == 0 {
        decode_additional_data(r)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct OplogMessageV0 {
    pub max_supported_version: u64,
    pub checkpoint: Option<Vec<Checkpointer>>,
    pub node: Node,
}

impl Codec for OplogMessageV0 {
    fn encoded_len(&self) -> usize {
        decode_only()
    }

    fn encode(&self, _: &mut Writer) {
        decode_only()
    }

    fn decode(r: &mut Reader) -> Result<Self> {
        let flags = r.uint()?;
        if flags & 1 != 0 {
            // DigestV0: a pointer and, for pointer 0, the indexer list.
            let pointer = r.uint()?;
            if pointer == 0 {
                let n = r.uint()?;
                for _ in 0..n {
                    let _ = r.uint()?;
                    let _ = r.fixed::<32>()?;
                    let _ = r.fixed::<32>()?;
                }
            }
        }
        let checkpoint = if flags & 1 != 0 {
            Some(decode_array::<Checkpointer>(r)?)
        } else {
            None
        };
        let node = Node::decode(r)?;
        decode_additional(r)?;
        let max_supported_version = if r.remaining() > 0 { r.uint()? } else { 0 };
        Ok(Self {
            max_supported_version,
            checkpoint,
            node,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OplogMessageV1 {
    pub max_supported_version: u64,
    pub digest: Option<[u8; 32]>,
    pub checkpoint: Option<Vec<Checkpointer>>,
    pub optimistic: bool,
    pub node: Node,
}

impl Codec for OplogMessageV1 {
    fn encoded_len(&self) -> usize {
        decode_only()
    }

    fn encode(&self, _: &mut Writer) {
        decode_only()
    }

    fn decode(r: &mut Reader) -> Result<Self> {
        let max_supported_version = r.uint()?;
        let flags = r.uint()?;
        let is_checkpointer = flags & 1 != 0;
        let checkpoint = if is_checkpointer {
            Some(decode_array::<Checkpointer>(r)?)
        } else {
            None
        };
        let digest = if is_checkpointer {
            let pointer = r.uint()?;
            Some(if pointer == 0 {
                r.fixed::<32>()?
            } else {
                [0u8; 32]
            })
        } else {
            None
        };
        let node = Node::decode(r)?;
        Ok(Self {
            max_supported_version,
            digest,
            checkpoint,
            optimistic: flags & 2 != 0,
            node,
        })
    }
}

/// Silences the unused writer import when the file changes shape.
#[allow(dead_code)]
fn _writer(_: &mut W) {}

#[allow(dead_code)]
fn _bytes(_: Bytes) {}
