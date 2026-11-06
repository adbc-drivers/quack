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

/// Pinned DuckDB logical type identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogicalTypeId {
    Invalid = 0,
    SqlNull = 1,
    Unknown = 2,
    Any = 3,
    Unbound = 4,
    Template = 5,
    Type = 6,
    Boolean = 10,
    TinyInt = 11,
    SmallInt = 12,
    Integer = 13,
    BigInt = 14,
    Date = 15,
    Time = 16,
    TimestampSec = 17,
    TimestampMs = 18,
    Timestamp = 19,
    TimestampNs = 20,
    Decimal = 21,
    Float = 22,
    Double = 23,
    Char = 24,
    Varchar = 25,
    Blob = 26,
    Interval = 27,
    UTinyInt = 28,
    USmallInt = 29,
    UInteger = 30,
    UBigInt = 31,
    TimestampTz = 32,
    TimestampTzNs = 33,
    TimeTz = 34,
    TimeNs = 35,
    Bit = 36,
    StringLiteral = 37,
    IntegerLiteral = 38,
    BigNum = 39,
    UHugeInt = 49,
    HugeInt = 50,
    Pointer = 51,
    Validity = 53,
    Uuid = 54,
    Geometry = 60,
    Struct = 100,
    List = 101,
    Map = 102,
    Table = 103,
    Enum = 104,
    LegacyAggregateState = 105,
    Lambda = 106,
    Union = 107,
    Array = 108,
    Variant = 109,
    Tuple = 110,
}

impl TryFrom<u8> for LogicalTypeId {
    type Error = u8;
    fn try_from(value: u8) -> Result<Self, u8> {
        match value {
            0 => Ok(Self::Invalid),
            1 => Ok(Self::SqlNull),
            2 => Ok(Self::Unknown),
            3 => Ok(Self::Any),
            4 => Ok(Self::Unbound),
            5 => Ok(Self::Template),
            6 => Ok(Self::Type),
            10 => Ok(Self::Boolean),
            11 => Ok(Self::TinyInt),
            12 => Ok(Self::SmallInt),
            13 => Ok(Self::Integer),
            14 => Ok(Self::BigInt),
            15 => Ok(Self::Date),
            16 => Ok(Self::Time),
            17 => Ok(Self::TimestampSec),
            18 => Ok(Self::TimestampMs),
            19 => Ok(Self::Timestamp),
            20 => Ok(Self::TimestampNs),
            21 => Ok(Self::Decimal),
            22 => Ok(Self::Float),
            23 => Ok(Self::Double),
            24 => Ok(Self::Char),
            25 => Ok(Self::Varchar),
            26 => Ok(Self::Blob),
            27 => Ok(Self::Interval),
            28 => Ok(Self::UTinyInt),
            29 => Ok(Self::USmallInt),
            30 => Ok(Self::UInteger),
            31 => Ok(Self::UBigInt),
            32 => Ok(Self::TimestampTz),
            33 => Ok(Self::TimestampTzNs),
            34 => Ok(Self::TimeTz),
            35 => Ok(Self::TimeNs),
            36 => Ok(Self::Bit),
            37 => Ok(Self::StringLiteral),
            38 => Ok(Self::IntegerLiteral),
            39 => Ok(Self::BigNum),
            49 => Ok(Self::UHugeInt),
            50 => Ok(Self::HugeInt),
            51 => Ok(Self::Pointer),
            53 => Ok(Self::Validity),
            54 => Ok(Self::Uuid),
            60 => Ok(Self::Geometry),
            100 => Ok(Self::Struct),
            101 => Ok(Self::List),
            102 => Ok(Self::Map),
            103 => Ok(Self::Table),
            104 => Ok(Self::Enum),
            105 => Ok(Self::LegacyAggregateState),
            106 => Ok(Self::Lambda),
            107 => Ok(Self::Union),
            108 => Ok(Self::Array),
            109 => Ok(Self::Variant),
            110 => Ok(Self::Tuple),
            _ => Err(value),
        }
    }
}

/// Strings retain arbitrary wire bytes rather than requiring UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalType {
    pub id: LogicalTypeId,
    pub info: Option<TypeInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInfo {
    pub alias: Vec<u8>,
    /// Whether an explicitly present, empty extension descriptor was received.
    pub extension_info: bool,
    pub kind: TypeInfoKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeInfoKind {
    Generic,
    Decimal {
        width: u8,
        scale: u8,
    },
    String {
        collation: Vec<u8>,
    },
    List {
        child: Box<LogicalType>,
    },
    Struct {
        children: Vec<(Vec<u8>, LogicalType)>,
    },
    Enum {
        values: Vec<Vec<u8>>,
    },
    Array {
        child: Box<LogicalType>,
        size: u32,
    },
    Any {
        target: Box<LogicalType>,
        cast_score: u64,
    },
    Template {
        name: Vec<u8>,
    },
    Geometry {
        crs: Vec<u8>,
    },
}
