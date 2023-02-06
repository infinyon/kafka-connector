use crate::config::KafkaConfig;
use anyhow::{Context, Result};
use async_trait::async_trait;
use core::iter::FromIterator;
use fluvio::consumer::Record;
use fluvio::Offset;
use fluvio_connector_common::{
    tracing::{debug, error, info},
    LocalBoxSink, Sink,
};
use rdkafka::{
    producer::{FutureProducer, FutureRecord},
    ClientConfig,
};

pub(crate) struct KafkaSink {
    topic: String,
    client_config: ClientConfig,
    create_topic: Option<bool>,
    partition: Option<i32>,
}

impl KafkaSink {
    pub(crate) fn new(config: &KafkaConfig) -> Result<Self> {
        let KafkaConfig {
            url,
            topic,
            partition,
            options,
            create_topic,
            security,
        } = config;

        let mut client_config = ClientConfig::from_iter(options.clone());
        client_config.set("bootstrap.servers", url.clone());
        if let Some(security) = security {
            if let Some(protocol) = &security.security_protocol {
                client_config.set("security.protocol", protocol.as_str());
            }
            if let Some(ssl_key) = &security.ssl_key {
                client_config.set("ssl.key.pem", ssl_key.content()?);
            }
            if let Some(ssl_cert) = &security.ssl_cert {
                client_config.set("ssl.certificate.pem", ssl_cert.content()?);
            }
            if let Some(ssl_ca) = &security.ssl_ca {
                client_config.set("ssl.ca.location", ssl_ca.path()?.to_string_lossy());
            }
        }
        Ok(Self {
            topic: topic.clone(),
            client_config,
            create_topic: *create_topic,
            partition: *partition,
        })
    }
}

#[async_trait]
impl Sink<Record> for KafkaSink {
    async fn connect(self, _offset: Option<Offset>) -> Result<LocalBoxSink<Record>> {
        if let Some(true) = self.create_topic {
            info!("creating kafka topic {} (if not existed)", &self.topic);
            create_kafka_topic(&self.topic, &self.client_config)
                .await
                .context("Unable to create topic on Kafka cluster")?;
        }
        debug!(?self.client_config);
        let kafka_producer: FutureProducer = self
            .client_config
            .create()
            .context("Unable to create Kafka producer")?;
        info!("Kafka producer created");

        let unfold = futures::sink::unfold(
            kafka_producer,
            move |kafka_producer: FutureProducer, record: Record| {
                let mut kafka_record = FutureRecord::to(self.topic.as_str())
                    .payload(record.value())
                    .key(record.key().unwrap_or(&[]));
                if let Some(kafka_partition) = self.partition {
                    kafka_record = kafka_record.partition(kafka_partition);
                }
                let enqueue_res = kafka_producer.send_result(kafka_record);
                if let Err((error, _)) = enqueue_res {
                    error!(
                        "KafkaError {:?}, offset: {}, partition: {}",
                        error, &record.offset, &record.partition
                    );
                }
                std::future::ready(Ok::<_, anyhow::Error>(kafka_producer))
            },
        );
        Ok(Box::pin(unfold))
    }
}

async fn create_kafka_topic(
    kafka_topic: &str,
    client_config: &rdkafka::ClientConfig,
) -> Result<()> {
    info!("Creating topic on kafka cluster");
    // Prepare topic, ensure it exists before creating producer
    use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
    use rdkafka::config::FromClientConfig;
    let admin = AdminClient::from_config(client_config)?;
    admin
        .create_topics(
            &[NewTopic::new(kafka_topic, 1, TopicReplication::Fixed(1))],
            &AdminOptions::new(),
        )
        .await?;
    Ok(())
}
