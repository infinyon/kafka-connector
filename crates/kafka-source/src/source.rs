use anyhow::{Context, Result};
use async_std::channel::{self, Sender};
use async_std::task::spawn;
use async_trait::async_trait;
use fluvio::{dataplane::record::RecordData, Offset, RecordKey};
use fluvio_connector_common::tracing::{info, trace};
use fluvio_connector_common::Source;
use futures::{stream::LocalBoxStream, StreamExt};
use kafka::consumer::{Consumer, FetchOffset, GroupOffsetStorage};

use crate::config::KafkaConfig;

const CHANNEL_BUFFER_SIZE: usize = 10000;

pub(crate) type Record = (RecordKey, RecordData);

#[derive(Debug)]
pub(crate) struct KafkaSource {
    consumer: Consumer,
}

impl KafkaSource {
    pub(crate) fn new(config: &KafkaConfig) -> Result<Self> {
        let KafkaConfig {
            url,
            group,
            topic,
            partition,
        } = config;
        let consumer = Consumer::from_hosts(vec![url.resolve()?])
            .with_topic_partitions(topic.clone(), &[*partition])
            .with_fallback_offset(FetchOffset::Earliest)
            .with_group(
                group
                    .clone()
                    .unwrap_or_else(|| "fluvio-kafka-source".to_string()),
            )
            .with_offset_storage(Some(GroupOffsetStorage::Kafka))
            .create()
            .context("Unable to create Kafka consumer")?;

        Ok(Self { consumer })
    }
}

#[async_trait]
impl<'a> Source<'a, Record> for KafkaSource {
    async fn connect(self, _offset: Option<Offset>) -> Result<LocalBoxStream<'a, Record>> {
        let (sender, receiver) = channel::bounded(CHANNEL_BUFFER_SIZE);
        spawn(kafka_loop(self.consumer, sender));
        Ok(receiver.boxed_local())
    }
}

async fn kafka_loop(mut consumer: Consumer, tx: Sender<Record>) -> Result<()> {
    info!("Kafka loop started");
    loop {
        for ms in consumer.poll().context("Kafka poll failed")?.iter() {
            trace!(
                "got {} messages, topic {}, partition {}",
                ms.messages().len(),
                ms.topic(),
                ms.partition()
            );
            for m in ms.messages() {
                tx.send((m.key.into(), m.value.into())).await?;
            }
            consumer
                .consume_messageset(ms)
                .context("Kafka message set consuming failed")?;
        }
        consumer.commit_consumed().context("Kafka commit failed")?;
    }
}
