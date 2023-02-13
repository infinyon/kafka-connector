use fluvio_connector_common::connector;

#[connector(config, name = "kafka")]
#[derive(Debug)]
pub(crate) struct KafkaConfig {
    pub url: String,

    pub group: Option<String>,

    pub topic: String,

    #[serde(default)]
    pub partition: i32,
}
