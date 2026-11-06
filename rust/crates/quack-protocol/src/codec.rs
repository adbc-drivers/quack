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

use crate::message::*;
use std::fmt;

const END: u16 = u16::MAX;

#[derive(Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub offset: usize,
    pub context: &'static str,
    pub message: String,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at byte {}: {}",
            self.context, self.offset, self.message
        )
    }
}

impl std::error::Error for DecodeError {}

#[derive(Debug, PartialEq, Eq)]
pub struct EncodeError {
    pub context: &'static str,
    pub message: &'static str,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.message)
    }
}

impl std::error::Error for EncodeError {}

/// Send `prefix` followed by `payload` as a single HTTP body. No extra framing.
#[derive(Debug)]
pub struct EncodedMessage<'a> {
    pub prefix: Vec<u8>,
    pub payload: &'a [u8],
}

impl EncodedMessage<'_> {
    /// Explicitly copy both segments into a contiguous allocation.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut bytes = self.prefix.clone();
        bytes.extend_from_slice(self.payload);
        bytes
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    context: &'static str,
}

impl<'a> Reader<'a> {
    fn error(&self, message: impl Into<String>) -> DecodeError {
        DecodeError {
            offset: self.pos,
            context: self.context,
            message: message.into(),
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| self.error("length overflow"))?;
        let result = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| self.error("truncated input"))?;
        self.pos = end;
        Ok(result)
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn peek_id(&self) -> Result<u16, DecodeError> {
        let bytes = self
            .bytes
            .get(self.pos..)
            .ok_or_else(|| self.error("invalid offset"))?;
        let pair = bytes
            .first_chunk::<2>()
            .ok_or_else(|| self.error("truncated field ID"))?;
        Ok(u16::from_le_bytes(*pair))
    }

    fn field<T: Default>(
        &mut self,
        id: u16,
        context: &'static str,
        required: bool,
        read: fn(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<T, DecodeError> {
        self.context = context;
        let actual = self.peek_id()?;
        if actual != id {
            if required {
                return Err(self.error(format!("expected field {id}, found {actual}")));
            }
            return Ok(T::default());
        }
        self.pos += 2;
        read(self)
    }

    fn end(&mut self, context: &'static str) -> Result<(), DecodeError> {
        self.context = context;
        let actual = self.peek_id()?;
        if actual != END {
            return Err(self.error(format!("expected object terminator, found field {actual}")));
        }
        self.pos += 2;
        Ok(())
    }

    fn unsigned(&mut self, width: u32) -> Result<u64, DecodeError> {
        let mut value = 0u128;
        for index in 0..width.div_ceil(7) {
            let byte = self.byte()?;
            value |= u128::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                if value >= (1u128 << width) {
                    return Err(self.error("unsigned LEB128 overflow"));
                }
                return Ok(value as u64);
            }
        }
        Err(self.error("unsigned LEB128 exceeds encoded width"))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        self.unsigned(64)
    }

    fn i64(&mut self) -> Result<i64, DecodeError> {
        let mut value = 0i128;
        for index in 0..10 {
            let byte = self.byte()?;
            value |= i128::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                if byte & 0x40 != 0 {
                    value |= !0i128 << ((index + 1) * 7);
                }
                return i64::try_from(value).map_err(|_| self.error("signed LEB128 overflow"));
            }
        }
        Err(self.error("signed LEB128 exceeds encoded width"))
    }

    fn boolean(&mut self) -> Result<bool, DecodeError> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.error("invalid boolean")),
        }
    }

    fn string(&mut self) -> Result<WireString, DecodeError> {
        let length = self.unsigned(32)?;
        let length = usize::try_from(length).map_err(|_| self.error("string length overflow"))?;
        Ok(WireString(self.take(length)?.to_vec()))
    }

    fn optional_idx(&mut self) -> Result<Option<u64>, DecodeError> {
        match self.u64()? {
            u64::MAX => Ok(None),
            value => Ok(Some(value)),
        }
    }

    fn hugeint(&mut self) -> Result<HugeInt, DecodeError> {
        Ok(HugeInt {
            upper: self.i64()?,
            lower: self.u64()?,
        })
    }

    fn logical_types(
        &mut self,
    ) -> Result<Vec<duckdb_wiredata::logical_type::LogicalType>, DecodeError> {
        let count = self.u64()?;
        if count > ((self.bytes.len() - self.pos) / 5) as u64 {
            return Err(self.error("type count exceeds remaining input"));
        }
        let mut types = Vec::new();
        for _ in 0..count {
            let (value, consumed) = duckdb_wiredata::decode::logical_type(&self.bytes[self.pos..])
                .map_err(|error| DecodeError {
                    offset: self.pos + error.offset,
                    context: error.context,
                    message: error.message,
                })?;
            self.pos += consumed;
            types.push(value);
        }
        Ok(types)
    }

    fn strings(&mut self) -> Result<Vec<WireString>, DecodeError> {
        let count = self.u64()?;
        if count > (self.bytes.len() - self.pos) as u64 {
            return Err(self.error("string count exceeds remaining input"));
        }
        let mut strings = Vec::new();
        for _ in 0..count {
            strings.push(self.string()?);
        }
        Ok(strings)
    }

    fn map(&mut self) -> Result<Vec<(WireString, WireString)>, DecodeError> {
        let count = self.u64()?;
        // Each entry needs two IDs, two string lengths, and a terminator.
        if count > ((self.bytes.len() - self.pos) / 8) as u64 {
            return Err(self.error("map count exceeds remaining input"));
        }
        let mut entries = Vec::new();
        for _ in 0..count {
            let key = self.field(0, "extra_info.key", true, Self::string)?;
            let value = self.field(1, "extra_info.value", true, Self::string)?;
            self.end("extra_info.entry")?;
            entries.push((key, value));
        }
        Ok(entries)
    }
}

type PrepareMetadata = (
    Vec<duckdb_wiredata::logical_type::LogicalType>,
    Vec<WireString>,
    bool,
    HugeInt,
);

pub(crate) fn decode_prepare_body(bytes: &[u8]) -> Result<PrepareMetadata, DecodeError> {
    let mut reader = Reader {
        bytes,
        pos: 0,
        context: "PrepareResponse",
    };
    let types = reader.field(
        1,
        "PrepareResponse.result_types",
        false,
        Reader::logical_types,
    )?;
    let names = reader.field(2, "PrepareResponse.result_names", false, Reader::strings)?;
    let more = reader.field(
        3,
        "PrepareResponse.needs_more_fetch",
        false,
        Reader::boolean,
    )?;
    let count = reader.field(4, "PrepareResponse.results", false, Reader::u64)?;
    if count != 0 {
        return Err(reader.error("nonempty inline results are unsupported; request inline_rows=0"));
    }
    let uuid = reader.field(5, "PrepareResponse.query_uuid", true, Reader::hugeint)?;
    reader.end("PrepareResponse")?;
    if reader.pos != bytes.len() {
        return Err(reader.error("trailing bytes after prepare response"));
    }
    Ok((types, names, more, uuid))
}

struct Writer {
    bytes: Vec<u8>,
    context: &'static str,
}

impl Writer {
    fn field<T: Default + PartialEq>(
        &mut self,
        id: u16,
        context: &'static str,
        required: bool,
        value: &T,
        write: fn(&mut Self, &T) -> Result<(), EncodeError>,
    ) -> Result<(), EncodeError> {
        self.context = context;
        if required || *value != T::default() {
            self.id(id);
            write(self, value)?;
        }
        Ok(())
    }

    fn id(&mut self, id: u16) {
        self.bytes.extend_from_slice(&id.to_le_bytes());
    }

    fn end(&mut self) {
        self.id(END);
    }

    fn u64(&mut self, value: &u64) -> Result<(), EncodeError> {
        let mut value = *value;
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            self.bytes.push(byte | if value == 0 { 0 } else { 0x80 });
            if value == 0 {
                return Ok(());
            }
        }
    }

    fn i64(&mut self, value: i64) {
        let mut value = value;
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
            self.bytes.push(byte | if done { 0 } else { 0x80 });
            if done {
                return;
            }
        }
    }

    fn boolean(&mut self, value: &bool) -> Result<(), EncodeError> {
        self.bytes.push(u8::from(*value));
        Ok(())
    }

    fn string(&mut self, value: &WireString) -> Result<(), EncodeError> {
        let length = u32::try_from(value.0.len()).map_err(|_| EncodeError {
            context: self.context,
            message: "string exceeds u32 byte length",
        })?;
        self.u64(&u64::from(length))?;
        self.bytes.extend_from_slice(&value.0);
        Ok(())
    }

    fn optional_idx(&mut self, value: &Option<u64>) -> Result<(), EncodeError> {
        match value {
            Some(u64::MAX) => Err(EncodeError {
                context: self.context,
                message: "u64::MAX is reserved for unset",
            }),
            Some(value) => self.u64(value),
            None => self.u64(&u64::MAX),
        }
    }

    fn hugeint(&mut self, value: &HugeInt) -> Result<(), EncodeError> {
        self.i64(value.upper);
        self.u64(&value.lower)
    }

    fn map(&mut self, entries: &[(WireString, WireString)]) -> Result<(), EncodeError> {
        self.u64(&(entries.len() as u64))?;
        for (key, value) in entries {
            self.field(0, "extra_info.key", true, key, Self::string)?;
            self.field(1, "extra_info.value", true, value, Self::string)?;
            self.end();
        }
        Ok(())
    }
}

fn payload_consistent(count: u64, bytes: &[u8]) -> bool {
    (count == 0) == bytes.is_empty()
}

/// Decode a complete HTTP body, reusing its allocation for any opaque payload.
/// Appended chunk payloads remain opaque. Prepare bodies require empty inline results.
pub fn decode(input: Vec<u8>) -> Result<Message, DecodeError> {
    let mut reader = Reader {
        bytes: &input,
        pos: 0,
        context: "header.type",
    };
    let id = reader.field(1, "header.type", true, |r| r.unsigned(8))?;
    let header = Header {
        connection_id: reader.field(2, "header.connection_id", false, Reader::string)?,
        client_query_id: reader.field(3, "header.client_query_id", true, Reader::optional_idx)?,
    };
    reader.end("header")?;
    if id == 4 {
        let start = reader.pos;
        let body =
            PrepareResponse::from_body(OpaqueBytes::tail(input, start)).map_err(|mut error| {
                error.offset += start;
                error
            })?;
        return Ok(Message {
            header,
            body: MessageBody::PrepareResponse(body),
        });
    }
    let mut body = match id {
        1 => MessageBody::ConnectionRequest(ConnectionRequest {
            auth_string: reader.field(1, "ConnectionRequest.auth_string", false, Reader::string)?,
            client_duckdb_version: reader.field(
                2,
                "ConnectionRequest.client_duckdb_version",
                false,
                Reader::string,
            )?,
            client_platform: reader.field(
                3,
                "ConnectionRequest.client_platform",
                false,
                Reader::string,
            )?,
            min_supported_quack_version: reader.field(
                4,
                "ConnectionRequest.min_supported_quack_version",
                false,
                Reader::u64,
            )?,
            max_supported_quack_version: reader.field(
                5,
                "ConnectionRequest.max_supported_quack_version",
                false,
                Reader::u64,
            )?,
            client_id: reader.field(6, "ConnectionRequest.client_id", false, Reader::string)?,
            heartbeat_timeout_seconds: reader.field(
                7,
                "ConnectionRequest.heartbeat_timeout_seconds",
                false,
                Reader::u64,
            )?,
        }),
        2 => MessageBody::ConnectionResponse(ConnectionResponse {
            server_duckdb_version: reader.field(
                1,
                "ConnectionResponse.server_duckdb_version",
                false,
                Reader::string,
            )?,
            server_platform: reader.field(
                2,
                "ConnectionResponse.server_platform",
                false,
                Reader::string,
            )?,
            quack_version: reader.field(
                3,
                "ConnectionResponse.quack_version",
                false,
                Reader::u64,
            )?,
            heartbeat_timeout_seconds: reader.field(
                4,
                "ConnectionResponse.heartbeat_timeout_seconds",
                false,
                Reader::u64,
            )?,
        }),
        3 => MessageBody::PrepareRequest(PrepareRequest {
            sql_query: reader.field(1, "PrepareRequest.sql_query", false, Reader::string)?,
            query_uuid: reader.field(2, "PrepareRequest.query_uuid", true, Reader::hugeint)?,
            inline_rows: reader.field(
                3,
                "PrepareRequest.inline_rows",
                false,
                Reader::optional_idx,
            )?,
        }),
        7 => MessageBody::FetchRequest(FetchRequest {
            query_uuid: reader.field(1, "FetchRequest.query_uuid", true, Reader::hugeint)?,
            batch_index: reader.field(2, "FetchRequest.batch_index", false, Reader::u64)?,
            ack_index: reader.field(3, "FetchRequest.ack_index", false, Reader::u64)?,
        }),
        8 => MessageBody::FetchResponse(FetchResponse {
            chunk_count: reader.field(1, "FetchResponse.chunk_count", false, Reader::u64)?,
            total_batches: reader.field(
                2,
                "FetchResponse.total_batches",
                false,
                Reader::optional_idx,
            )?,
            batch_index: reader.field(
                3,
                "FetchResponse.batch_index",
                false,
                Reader::optional_idx,
            )?,
            payload: OpaqueBytes::default(),
        }),
        9 => MessageBody::SendDataRequest(SendDataRequest {
            stream_id: reader.field(1, "SendDataRequest.stream_id", false, Reader::string)?,
            chunk_count: reader.field(2, "SendDataRequest.chunk_count", false, Reader::u64)?,
            total_batches: reader.field(
                3,
                "SendDataRequest.total_batches",
                false,
                Reader::optional_idx,
            )?,
            batch_index: reader.field(
                4,
                "SendDataRequest.batch_index",
                false,
                Reader::optional_idx,
            )?,
            payload: OpaqueBytes::default(),
        }),
        10 => MessageBody::SuccessResponse(SuccessResponse),
        11 => MessageBody::DisconnectMessage(DisconnectMessage),
        12 => MessageBody::CancelRequest(CancelRequest {
            query_uuid: reader.field(1, "CancelRequest.query_uuid", true, Reader::hugeint)?,
        }),
        14 => MessageBody::SendDataResponse(SendDataResponse {
            accept_budget: reader.field(
                1,
                "SendDataResponse.accept_budget",
                true,
                Reader::optional_idx,
            )?,
        }),
        15 => MessageBody::Acknowledgement(Acknowledgement {
            query_uuid: reader.field(1, "Acknowledgement.query_uuid", false, Reader::hugeint)?,
        }),
        16 => MessageBody::HeartbeatRequest(HeartbeatRequest),
        100 => MessageBody::ErrorResponse(ErrorResponse {
            message: reader.field(1, "ErrorResponse.message", false, Reader::string)?,
            exception_type: reader.field(
                2,
                "ErrorResponse.exception_type",
                false,
                Reader::string,
            )?,
            extra_info: reader.field(3, "ErrorResponse.extra_info", false, Reader::map)?,
            must_invalidate: reader.field(
                4,
                "ErrorResponse.must_invalidate",
                false,
                Reader::boolean,
            )?,
        }),
        _ => {
            return Err(DecodeError {
                offset: 2,
                context: "header.type",
                message: format!("unsupported message ID {id}"),
            });
        }
    };
    reader.end("body")?;
    let start = reader.pos;
    match &mut body {
        MessageBody::FetchResponse(value) => {
            if !payload_consistent(value.chunk_count, &input[start..]) {
                return Err(reader.error("chunk count and payload emptiness disagree"));
            }
            value.payload = OpaqueBytes::tail(input, start);
        }
        MessageBody::SendDataRequest(value) => {
            if !payload_consistent(value.chunk_count, &input[start..]) {
                return Err(reader.error("chunk count and payload emptiness disagree"));
            }
            value.payload = OpaqueBytes::tail(input, start);
        }
        _ => {
            if start != input.len() {
                return Err(reader.error("unexpected trailing bytes"));
            }
        }
    }
    Ok(Message { header, body })
}

/// Serialize metadata and borrow opaque bytes without copying their contents.
pub fn encode(message: &Message) -> Result<EncodedMessage<'_>, EncodeError> {
    let mut writer = Writer {
        bytes: Vec::new(),
        context: "header",
    };
    writer.field(
        1,
        "header.type",
        true,
        &u64::from(message.body.message_id()),
        Writer::u64,
    )?;
    writer.field(
        2,
        "header.connection_id",
        false,
        &message.header.connection_id,
        Writer::string,
    )?;
    writer.field(
        3,
        "header.client_query_id",
        true,
        &message.header.client_query_id,
        Writer::optional_idx,
    )?;
    writer.end();
    let mut payload: &[u8] = &[];
    match &message.body {
        MessageBody::PrepareResponse(value) => {
            let payload = value.body().as_bytes();
            return Ok(EncodedMessage {
                prefix: writer.bytes,
                payload,
            });
        }
        MessageBody::ConnectionRequest(value) => {
            writer.field(
                1,
                "ConnectionRequest.auth_string",
                false,
                &value.auth_string,
                Writer::string,
            )?;
            writer.field(
                2,
                "ConnectionRequest.client_duckdb_version",
                false,
                &value.client_duckdb_version,
                Writer::string,
            )?;
            writer.field(
                3,
                "ConnectionRequest.client_platform",
                false,
                &value.client_platform,
                Writer::string,
            )?;
            writer.field(
                4,
                "ConnectionRequest.min_supported_quack_version",
                false,
                &value.min_supported_quack_version,
                Writer::u64,
            )?;
            writer.field(
                5,
                "ConnectionRequest.max_supported_quack_version",
                false,
                &value.max_supported_quack_version,
                Writer::u64,
            )?;
            writer.field(
                6,
                "ConnectionRequest.client_id",
                false,
                &value.client_id,
                Writer::string,
            )?;
            writer.field(
                7,
                "ConnectionRequest.heartbeat_timeout_seconds",
                false,
                &value.heartbeat_timeout_seconds,
                Writer::u64,
            )?;
        }
        MessageBody::ConnectionResponse(value) => {
            writer.field(
                1,
                "ConnectionResponse.server_duckdb_version",
                false,
                &value.server_duckdb_version,
                Writer::string,
            )?;
            writer.field(
                2,
                "ConnectionResponse.server_platform",
                false,
                &value.server_platform,
                Writer::string,
            )?;
            writer.field(
                3,
                "ConnectionResponse.quack_version",
                false,
                &value.quack_version,
                Writer::u64,
            )?;
            writer.field(
                4,
                "ConnectionResponse.heartbeat_timeout_seconds",
                false,
                &value.heartbeat_timeout_seconds,
                Writer::u64,
            )?;
        }
        MessageBody::PrepareRequest(value) => {
            writer.field(
                1,
                "PrepareRequest.sql_query",
                false,
                &value.sql_query,
                Writer::string,
            )?;
            writer.field(
                2,
                "PrepareRequest.query_uuid",
                true,
                &value.query_uuid,
                Writer::hugeint,
            )?;
            writer.field(
                3,
                "PrepareRequest.inline_rows",
                false,
                &value.inline_rows,
                Writer::optional_idx,
            )?;
        }
        MessageBody::FetchRequest(value) => {
            writer.field(
                1,
                "FetchRequest.query_uuid",
                true,
                &value.query_uuid,
                Writer::hugeint,
            )?;
            writer.field(
                2,
                "FetchRequest.batch_index",
                false,
                &value.batch_index,
                Writer::u64,
            )?;
            writer.field(
                3,
                "FetchRequest.ack_index",
                false,
                &value.ack_index,
                Writer::u64,
            )?;
        }
        MessageBody::FetchResponse(value) => {
            writer.field(
                1,
                "FetchResponse.chunk_count",
                false,
                &value.chunk_count,
                Writer::u64,
            )?;
            writer.field(
                2,
                "FetchResponse.total_batches",
                false,
                &value.total_batches,
                Writer::optional_idx,
            )?;
            writer.field(
                3,
                "FetchResponse.batch_index",
                false,
                &value.batch_index,
                Writer::optional_idx,
            )?;
            payload = value.payload.as_bytes();
            if !payload_consistent(value.chunk_count, payload) {
                return Err(EncodeError {
                    context: "FetchResponse.payload",
                    message: "chunk count and payload emptiness disagree",
                });
            }
        }
        MessageBody::SendDataRequest(value) => {
            writer.field(
                1,
                "SendDataRequest.stream_id",
                false,
                &value.stream_id,
                Writer::string,
            )?;
            writer.field(
                2,
                "SendDataRequest.chunk_count",
                false,
                &value.chunk_count,
                Writer::u64,
            )?;
            writer.field(
                3,
                "SendDataRequest.total_batches",
                false,
                &value.total_batches,
                Writer::optional_idx,
            )?;
            writer.field(
                4,
                "SendDataRequest.batch_index",
                false,
                &value.batch_index,
                Writer::optional_idx,
            )?;
            payload = value.payload.as_bytes();
            if !payload_consistent(value.chunk_count, payload) {
                return Err(EncodeError {
                    context: "SendDataRequest.payload",
                    message: "chunk count and payload emptiness disagree",
                });
            }
        }
        MessageBody::SuccessResponse(_) => {}
        MessageBody::DisconnectMessage(_) => {}
        MessageBody::CancelRequest(value) => {
            writer.field(
                1,
                "CancelRequest.query_uuid",
                true,
                &value.query_uuid,
                Writer::hugeint,
            )?;
        }
        MessageBody::SendDataResponse(value) => {
            writer.field(
                1,
                "SendDataResponse.accept_budget",
                true,
                &value.accept_budget,
                Writer::optional_idx,
            )?;
        }
        MessageBody::Acknowledgement(value) => {
            writer.field(
                1,
                "Acknowledgement.query_uuid",
                false,
                &value.query_uuid,
                Writer::hugeint,
            )?;
        }
        MessageBody::HeartbeatRequest(_) => {}
        MessageBody::ErrorResponse(value) => {
            writer.field(
                1,
                "ErrorResponse.message",
                false,
                &value.message,
                Writer::string,
            )?;
            writer.field(
                2,
                "ErrorResponse.exception_type",
                false,
                &value.exception_type,
                Writer::string,
            )?;
            writer.field(
                3,
                "ErrorResponse.extra_info",
                false,
                &value.extra_info,
                |w, v| w.map(v),
            )?;
            writer.field(
                4,
                "ErrorResponse.must_invalidate",
                false,
                &value.must_invalidate,
                Writer::boolean,
            )?;
        }
    }
    writer.end();
    Ok(EncodedMessage {
        prefix: writer.bytes,
        payload,
    })
}
