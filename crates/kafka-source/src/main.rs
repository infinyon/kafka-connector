use anyhow::Result;
use config::KafkaConfig;
use fluvio::TopicProducer;
use fluvio_connector_common::{
    connector,
    tracing::{debug, trace},
    Source,
};
use futures::StreamExt;
use source::KafkaSource;

mod config;
mod source;

#[connector(source)]
async fn start(config: KafkaConfig, producer: TopicProducer) -> Result<()> {
    debug!(?config);
    let source = KafkaSource::new(&config)?;
    let mut stream = source.connect(None).await?;
    while let Some((key, value)) = stream.next().await {
        trace!(?value);
        producer.send(key, value).await?;
    }
    Ok(())
}
