use std::{collections::HashMap, path::PathBuf};

use anyhow::Context;
use fluvio_connector_common::connector;
use serde::Deserialize;

#[connector(config, name = "kafka")]
pub(crate) struct KafkaConfig {
    /// A Comma separated list of the kafka brokers to connect to
    pub url: String,

    /// The kafka topic to use
    pub topic: String,

    /// The kafka partition to use. This is optional
    pub partition: Option<i32>,

    /// Additional kafka options
    #[serde(default)]
    pub options: HashMap<String, String>,

    /// Boolean to create a kafka topic. Defaults to `false`
    #[serde(rename = "create-topic")]
    pub create_topic: Option<bool>,

    pub security: Option<Security>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Security {
    /// The SSL key file to use
    pub ssl_key: Option<SecurityFile>,

    /// The SSL cert file to use
    pub ssl_cert: Option<SecurityFile>,

    /// The SSL ca file to use
    pub ssl_ca: Option<SecurityFile>,

    /// The kafka security protocol. Currently only supports `SSL`.
    pub security_protocol: Option<SecurityProtocol>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecurityProtocol {
    Ssl,
    // TODO: SASL_SSL and SASL_PLAINTEXT
}

impl SecurityProtocol {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            SecurityProtocol::Ssl => "ssl",
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecurityFile {
    File(PathBuf),
    Pem(String),
}

impl SecurityFile {
    pub(crate) fn content(&self) -> anyhow::Result<String> {
        match self {
            Self::File(path) => std::fs::read_to_string(path).context(format!(
                "Unable to read content from file {}",
                path.to_string_lossy()
            )),
            Self::Pem(content) => Ok(content.clone()),
        }
    }

    pub(crate) fn path(&self) -> anyhow::Result<PathBuf> {
        use std::io::Write;
        match self {
            Self::File(path) => Ok(path.clone()),
            Self::Pem(content) => {
                let mut tmpfile =
                    tempfile::NamedTempFile::new().context("Unable to create temp file")?;
                write!(tmpfile, "{content}").context("Unable to write content to temp file")?;
                let (_file, path) = tmpfile.keep()?;
                Ok(path)
            }
        }
    }
}
