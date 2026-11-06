// Copyright (c) 2026 ADBC Drivers Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//         http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Segmented DuckDB serialization. Payload buffers retain their original owners.

use crate::logical_type::{LogicalType, TypeInfoKind};
use bytes::Bytes;

#[derive(Default)]
pub struct Encoder {
    metadata: Vec<u8>,
    segments: Vec<Bytes>,
}

impl Encoder {
    pub fn field(&mut self, id: u16) {
        self.metadata.extend_from_slice(&id.to_le_bytes());
    }
    pub fn end(&mut self) {
        self.field(u16::MAX);
    }
    pub fn uint(&mut self, mut value: u64) {
        while value >= 128 {
            self.metadata.push((value as u8 & 127) | 128);
            value >>= 7;
        }
        self.metadata.push(value as u8);
    }
    pub fn boolean(&mut self, value: bool) {
        self.metadata.push(u8::from(value));
    }
    /// Append an owned shared view without copying the bytes.
    pub fn raw(&mut self, bytes: Bytes) {
        self.flush();
        if !bytes.is_empty() {
            self.segments.push(bytes);
        }
    }
    pub fn blob(&mut self, bytes: Bytes) {
        self.uint(bytes.len() as u64);
        self.raw(bytes);
    }
    /// Encode the scalar descriptors supported by the ingestion encoder.
    pub fn logical_type(&mut self, value: &LogicalType) -> Result<(), &'static str> {
        if let Some(info) = &value.info {
            if !info.alias.is_empty() || info.extension_info {
                return Err("aliased or extended ingestion types are unsupported");
            }
            if !matches!(info.kind, TypeInfoKind::Decimal { .. }) {
                return Err("unsupported ingestion type metadata");
            }
        }
        self.field(100);
        self.uint(value.id as u64);
        if let Some(info) = &value.info {
            self.field(101);
            self.boolean(true);
            self.field(100);
            self.uint(2);
            if let TypeInfoKind::Decimal { width, scale } = info.kind {
                self.field(200);
                self.uint(u64::from(width));
                self.field(201);
                self.uint(u64::from(scale));
            }
            self.end();
        }
        self.end();
        Ok(())
    }
    fn flush(&mut self) {
        if !self.metadata.is_empty() {
            self.segments
                .push(std::mem::take(&mut self.metadata).into());
        }
    }
    pub fn finish(mut self) -> Vec<Bytes> {
        self.flush();
        self.segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{chunk, logical_type::LogicalTypeId};

    #[test]
    fn primitive_payload_is_shared_and_decodes() {
        let values = Bytes::from(vec![42, 0, 0, 0, 7, 0, 0, 0]);
        let mut encoder = Encoder::default();
        encoder.field(100);
        encoder.uint(2);
        encoder.field(101);
        encoder.uint(1);
        encoder
            .logical_type(&LogicalType {
                id: LogicalTypeId::Integer,
                info: None,
            })
            .unwrap();
        encoder.field(102);
        encoder.uint(1);
        encoder.field(100);
        encoder.boolean(false);
        encoder.field(102);
        encoder.blob(values.clone());
        encoder.end();
        encoder.end();
        let segments = encoder.finish();
        assert!(segments.iter().any(|s| s.as_ptr() == values.as_ptr()));
        let bytes: Vec<u8> = segments.iter().flat_map(|s| s.iter().copied()).collect();
        let (chunk, consumed) = chunk::decode(bytes.clone()).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(chunk.columns()[0].get::<i32>(0), Some(Some(42)));
        assert_eq!(chunk.columns()[0].get::<i32>(1), Some(Some(7)));
    }
}
