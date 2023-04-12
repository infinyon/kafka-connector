use fluvio_connector_common::{connector, secret::SecretString};

#[connector(config, name = "kafka")]
#[derive(Debug)]
pub(crate) struct KafkaConfig {
    pub url: SecretString,

    pub group: Option<String>,

    pub topic: String,

    #[serde(default)]
    pub partition: i32,
}
