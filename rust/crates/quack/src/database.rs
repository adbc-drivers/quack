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

use crate::connection::{QuackConnection, Session};
use crate::error::ErrorHelper;
use adbc_core::error::Result;
use adbc_core::options::{OptionConnection, OptionDatabase, OptionValue};
use adbc_core::{Database, Optionable};
use driverbase::error::ErrorHelper as _;

pub struct QuackDatabase {
    runtime: std::sync::Arc<tokio::runtime::Runtime>,
    uri: Option<url::Url>,
    tls: Option<String>,
}

impl QuackDatabase {
    pub(crate) fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                ErrorHelper::io()
                    .context("create runtime")
                    .message(e.to_string())
                    .to_adbc()
            })?;
        Ok(Self {
            runtime: std::sync::Arc::new(runtime),
            uri: None,
            tls: None,
        })
    }
}

fn invalid(message: impl AsRef<str>) -> adbc_core::error::Error {
    ErrorHelper::invalid_argument().message(message).to_adbc()
}

fn tls(value: &str) -> Result<quack_tokio::TlsMode> {
    match value {
        "true" => Ok(quack_tokio::TlsMode::Verify),
        "false" => Ok(quack_tokio::TlsMode::Disabled),
        "skip_verify" | "skip-verify" => Ok(quack_tokio::TlsMode::SkipVerify),
        _ => Err(invalid("invalid quack TLS mode")),
    }
}

fn query_options(uri: &url::Url) -> Result<(String, String)> {
    let mut token = None;
    let mut mode = None;
    for (key, value) in uri.query_pairs() {
        match key.as_ref() {
            "token" if token.is_none() => token = Some(value.into_owned()),
            "tls" => {
                if mode.is_some() {
                    return Err(invalid("duplicate quack URI tls parameter"));
                }
                tls(&value)?;
                mode = Some(value.into_owned());
            }
            _ => {}
        }
    }
    Ok((
        token.unwrap_or_default(),
        mode.unwrap_or_else(|| "true".into()),
    ))
}

impl Database for QuackDatabase {
    type ConnectionType = QuackConnection;

    fn new_connection(&self) -> Result<Self::ConnectionType> {
        self.new_connection_with_opts(vec![])
    }

    fn new_connection_with_opts(
        &self,
        opts: impl IntoIterator<Item = (OptionConnection, OptionValue)>,
    ) -> Result<Self::ConnectionType> {
        if let Some((key, _)) = opts.into_iter().next() {
            return Err(ErrorHelper::set_unknown_option(&key).to_adbc());
        }
        let uri = self
            .uri
            .as_ref()
            .ok_or_else(|| invalid("database option uri is required"))?;
        let (token, mode) = query_options(uri)?;
        let mode = self.tls.as_deref().unwrap_or(&mode);
        let transport = tls(mode)?;
        let scheme = if mode == "false" { "http" } else { "https" };
        let mut endpoint = url::Url::parse(&format!("{scheme}://localhost/quack"))
            .map_err(|e| invalid(e.to_string()))?;
        endpoint
            .set_host(uri.host_str())
            .map_err(|e| invalid(e.to_string()))?;
        endpoint
            .set_port(uri.port())
            .map_err(|_| invalid("invalid port"))?;
        let runtime = std::sync::Arc::clone(&self.runtime);
        let connection = runtime
            .block_on(quack_tokio::Connection::new(
                quack_tokio::ConnectionOptions {
                    endpoint: endpoint.into(),
                    tls: transport,
                    protocol: quack_protocol::state::ConnectionOptions {
                        auth_string: token.into(),
                        ..Default::default()
                    },
                },
            ))
            .map_err(|e| {
                let helper = if e.is::<tokio::time::error::Elapsed>() {
                    ErrorHelper::timeout()
                } else if e.is::<quack_tokio::RemoteError>() {
                    ErrorHelper::unknown()
                } else if e.is::<quack_protocol::state::Error>() {
                    ErrorHelper::invalid_data()
                } else {
                    ErrorHelper::io()
                };
                helper
                    .context("connect to Quack")
                    .message(e.to_string())
                    .to_adbc()
            })?;
        Ok(QuackConnection {
            manual: false,
            session: std::sync::Arc::new(Session {
                connection: std::sync::Mutex::new(connection),
                runtime,
            }),
        })
    }
}

impl Optionable for QuackDatabase {
    type Option = OptionDatabase;

    fn set_option(&mut self, key: Self::Option, value: OptionValue) -> Result<()> {
        match (&key, value) {
            (OptionDatabase::Uri, OptionValue::String(value)) => {
                let uri = url::Url::parse(&value).map_err(|e| invalid(e.to_string()))?;
                if uri.scheme() != "quack" || uri.host_str().is_none() {
                    return Err(invalid("expected quack URI with host"));
                }
                query_options(&uri)?;
                self.uri = Some(uri);
                Ok(())
            }
            (OptionDatabase::Other(name), OptionValue::String(value)) if name == "quack.tls" => {
                tls(&value)?;
                self.tls = Some(value);
                Ok(())
            }
            (OptionDatabase::Uri, _) => Err(invalid("uri must be a string")),
            (OptionDatabase::Other(name), _) if name == "quack.tls" => {
                Err(invalid("quack.tls must be a string"))
            }
            _ => Err(ErrorHelper::set_unknown_option(&key).to_adbc()),
        }
    }

    fn get_option_string(&self, key: Self::Option) -> Result<String> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }

    fn get_option_bytes(&self, key: Self::Option) -> Result<Vec<u8>> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }

    fn get_option_int(&self, key: Self::Option) -> Result<i64> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }

    fn get_option_double(&self, key: Self::Option) -> Result<f64> {
        Err(ErrorHelper::get_unknown_option(&key).to_adbc())
    }
}
