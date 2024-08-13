use anyhow::Result;
use config::KafkaConfig;
use futures::StreamExt;

use fluvio::TopicProducerPool;
use fluvio_connector_common::{
    connector,
    tracing::{debug, trace},
    Source,
};

mod config;
mod source;
use source::KafkaSource;

#[connector(source)]
async fn start(config: KafkaConfig, producer: TopicProducerPool) -> Result<()> {
    debug!(?config);
    let source = KafkaSource::new(&config)?;
    let mut stream = source.connect(None).await?;
    while let Some((key, value)) = stream.next().await {
        trace!(?value);
        producer.send(key, value).await?;
    }
    Ok(())
}
