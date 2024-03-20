use anyhow::{Context, Result};
use async_std::channel::{self, Sender};
use async_std::task::spawn;
use async_trait::async_trait;
use fluvio::{dataplane::record::RecordData, Offset, RecordKey};
use fluvio_connector_common::tracing::{error, info, trace};
use fluvio_connector_common::Source;
use futures::{stream::LocalBoxStream, StreamExt};
use kafka::consumer::{Consumer, FetchOffset, GroupOffsetStorage};

use crate::config::KafkaConfig;

const CHANNEL_BUFFER_SIZE: usize = 10000;
const DEFAULT_FETCH_MAX_BYTES_PER_PARTITION: i32 = 1024 * 1024;

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
            fetch_max_bytes_per_partition,
        } = config;

        let consumer = Consumer::from_hosts(vec![url.resolve()?])
            .with_topic_partitions(topic.clone(), &[*partition])
            .with_fallback_offset(FetchOffset::Earliest)
            .with_group(
                group
                    .clone()
                    .unwrap_or_else(|| "fluvio-kafka-source".to_string()),
            )
            .with_offset_storage(GroupOffsetStorage::Kafka)
            .with_fetch_max_bytes_per_partition(
                fetch_max_bytes_per_partition.unwrap_or(DEFAULT_FETCH_MAX_BYTES_PER_PARTITION),
            )
            .create()
            .context("Unable to create Kafka consumer")?;

        Ok(Self { consumer })
    }
}

#[async_trait]
impl<'a> Source<'a, Record> for KafkaSource {
    async fn connect(self, _offset: Option<Offset>) -> Result<LocalBoxStream<'a, Record>> {
        let (sender, receiver) = channel::bounded(CHANNEL_BUFFER_SIZE);
        spawn(kafka_loop_wrapper(self.consumer, sender));
        Ok(receiver.boxed_local())
    }
}

async fn kafka_loop_wrapper(consumer: Consumer, tx: Sender<Record>) {
    if let Err(err) = kafka_loop(consumer, tx).await {
        error!("Kafka loop failed: {}", err);
    }
}

async fn kafka_loop(mut consumer: Consumer, tx: Sender<Record>) -> Result<()> {
    info!("Kafka loop started");
    loop {
        for ms in consumer.poll()?.iter() {
            trace!(
                "got {} messages, topic {}, partition {}",
                ms.messages().len(),
                ms.topic(),
                ms.partition()
            );
            for m in ms.messages() {
                tx.send((m.key.into(), m.value.into())).await?;
            }
            consumer.consume_messageset(ms)?
        }
        consumer.commit_consumed()?
    }
}
