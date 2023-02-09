mod config;
mod sink;

use config::KafkaConfig;
use fluvio_connector_common::{connector, consumer::ConsumerStream, tracing::trace, Result, Sink};
use futures::SinkExt;
use sink::KafkaSink;

#[connector(sink)]
async fn start(config: KafkaConfig, mut stream: impl ConsumerStream) -> Result<()> {
    let sink = KafkaSink::new(&config)?;
    let mut sink = sink.connect(None).await?;
    while let Some(item) = stream.next().await {
        let record = item?;
        trace!(?record.offset, ?record.partition);
        sink.send(record).await?;
    }
    Ok(())
}
