use std::time::Duration;

use anyhow::{Context, Result};
use async_std::channel::{self, Sender};
use async_std::task::spawn;
use async_trait::async_trait;
use fluvio::{dataplane::record::RecordData, Offset, RecordKey};
use fluvio_connector_common::tracing::{error, info, trace, warn};
use fluvio_connector_common::Source;
use futures::{stream::LocalBoxStream, StreamExt};
use kafka::consumer::{Consumer, FetchOffset, GroupOffsetStorage};

use crate::backoff::Backoff;
use crate::config::KafkaConfig;

const CHANNEL_BUFFER_SIZE: usize = 10000;
const DEFAULT_FETCH_MAX_BYTES_PER_PARTITION: i32 = 1024 * 1024;
const BACKOFF_LIMIT: Duration = Duration::from_secs(1000);

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
        spawn(kafka_loop(self.consumer, sender));
        Ok(receiver.boxed_local())
    }
}

async fn kafka_loop(mut consumer: Consumer, tx: Sender<Record>) -> Result<()> {
    let mut backoff = Backoff::new();
    info!("Kafka loop started");
    loop {
        kafka_poll_with_backoff(&mut consumer, &tx, &mut backoff).await?
    }
}

async fn kafka_poll_with_backoff(
    consumer: &mut Consumer,
    tx: &Sender<Record>,
    backoff: &mut Backoff,
) -> Result<()> {
    let wait = backoff.next();

    if wait > BACKOFF_LIMIT {
        error!("Max retry reached, exiting");
    }

    match kafka_poll(consumer, tx).await {
        Ok(()) => Ok(()),
        Err(err) => {
            warn!(
                "Error polling from Kafka: \"{}\", retrying in {}.",
                err,
                humantime::format_duration(wait)
            );

            async_std::task::sleep(wait).await;

            Err(err)
        }
    }
}

async fn kafka_poll(consumer: &mut Consumer, tx: &Sender<Record>) -> Result<()> {
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
    consumer.commit_consumed()?;
    Ok(())
}
