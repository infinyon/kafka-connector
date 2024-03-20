use adaptive_backoff::prelude::*;
use std::time::Duration;

use adaptive_backoff::backoff::ExponentialBackoffBuilder;
use anyhow::{Context, Result};
use async_std::channel::{self, Sender};
use async_std::task::spawn;
use async_trait::async_trait;
use fluvio::{dataplane::record::RecordData, Offset, RecordKey};
use fluvio_connector_common::tracing::{info, trace, warn};
use fluvio_connector_common::Source;
use futures::{stream::LocalBoxStream, StreamExt};
use kafka::consumer::{Consumer, FetchOffset, GroupOffsetStorage};

use crate::config::KafkaConfig;

const CHANNEL_BUFFER_SIZE: usize = 10000;
const DEFAULT_FETCH_MAX_BYTES_PER_PARTITION: i32 = 1024 * 1024;
const BACKOFF_MIN: Duration = Duration::from_secs(0);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
const BACKOFF_FACTOR: f64 = 2.0;
const BACKOFF_LIMIT: Duration = Duration::from_secs(1024);

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
            .with_offset_storage(Some(GroupOffsetStorage::Kafka))
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
    let mut backoff = ExponentialBackoffBuilder::default()
        .min(BACKOFF_MIN)
        .max(BACKOFF_MAX)
        .factor(BACKOFF_FACTOR)
        .build()?;
    info!("Kafka loop started");
    loop {
        if let Ok(()) = kafka_poll_with_backoff(&mut consumer, &tx, &mut backoff).await {
            backoff.reset();
        };
    }
}

async fn kafka_poll_with_backoff(
    consumer: &mut Consumer,
    tx: &Sender<Record>,
    backoff: &mut ExponentialBackoff,
) -> Result<()> {
    let wait = backoff.wait();

    if wait > BACKOFF_LIMIT {
        panic!(
            "Backoff limit reached, waiting for {} before retrying.",
            humantime::format_duration(wait)
        );
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
