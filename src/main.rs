use atmb_us_physical::atmb::model::Mailbox;
use atmb_us_physical::atmb::ATMBCrawl;
use atmb_us_physical::record::Record;
use atmb_us_physical::smarty::{AdditionalInfo, SmartyClientProxy};
use futures::StreamExt;
use log::{error, info};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

fn init_logger() {
    let guard = install_tracing();
    // keep guard alive for the lifetime of the program
    let _ = LOG_GUARD.set(guard);
    color_eyre::install().unwrap();
}

fn install_tracing() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_appender::non_blocking;
    use tracing_appender::rolling;
    use tracing_error::ErrorLayer;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    // file log: result/run.log
    // ensure log directory exists
    let _ = std::fs::create_dir_all("result");
    let file_appender = rolling::never("result", "run.log");
    let (file_writer, guard) = non_blocking(file_appender);

    let fmt_layer = fmt::layer().with_target(false);
    let file_layer = fmt::layer()
        .with_target(false)
        .with_ansi(false)
        .event_format(fmt::format().compact())
        .with_writer(file_writer);
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .with(file_layer)
        .with(ErrorLayer::default())
        .init();

    guard
}

#[tokio::main]
async fn main() {
    init_logger();

    match run().await {
        Err(e) => {
            log::error!("Error: {:?}", e);
            std::process::exit(1);
        }
        _ => {}
    }
}

async fn run() -> color_eyre::Result<()> {
    let atmb = ATMBCrawl::new()?;
    let crawl_result = atmb.fetch().await?;
    let mailboxes = crawl_result.mailboxes;

    info!(
        "finished fetching, got [{}] mailboxes in total",
        mailboxes.len()
    );
    info!("begin to inquire mailbox address info...");

    let mailboxes_info = inquire_mailboxes_info(mailboxes).await?;
    // filter out CMRA and addresses
    let records = mailboxes_info
        .into_iter()
        .filter_map(|(mailbox, info)| {
            if info.is_cmra() {
                None
            } else {
                Some(Record::from_mailbox_and_info(mailbox, info))
            }
        })
        .collect::<Vec<_>>();

    let out_file = "result/mailboxes.csv";
    info!("saving records to [{}]", out_file);
    save_records(records, out_file)?;
    Ok(())
}

async fn inquire_mailboxes_info(
    mailboxes: Vec<Mailbox>,
) -> color_eyre::Result<HashMap<Mailbox, AdditionalInfo>> {
    let client = SmartyClientProxy::new()?;

    let total = mailboxes.len();
    let mailboxes_info = futures::stream::iter(mailboxes.into_iter())
        .enumerate()
        .map(|(idx, mailbox)| {
            let client = &client;
            async move {
                info!(
                    "[{}/{total}] fetching mailbox address info for [{}]",
                    idx + 1,
                    mailbox.name
                );

                let address = &mailbox.address;
                let additional_info = match client.inquire_address(address.clone()).await {
                    Ok(info) => info,
                    Err(e) => {
                        error!(
                            "cannot inquire address info for [{}]: {:?}",
                            mailbox.name, e
                        );
                        return None;
                    }
                };
                Some((mailbox, additional_info))
            }
        })
        // keep concurrency modest to avoid hammering Smarty free-tier limits
        .buffer_unordered(3)
        .collect::<Vec<_>>()
        .await;

    Ok(mailboxes_info
        .into_iter()
        .filter_map(|info| info)
        .collect::<HashMap<_, _>>())
}

/// write result to CSV file
fn save_records(mut records: Vec<Record>, save_path: impl AsRef<Path>) -> color_eyre::Result<()> {
    records.sort_by(|r1, r2| (&r1.cmra, &r1.rdi).cmp(&(&r2.cmra, &r2.rdi)));
    if let Some(parent) = save_path.as_ref().parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut wtr = csv::Writer::from_path(save_path)?;
    for record in &records {
        wtr.serialize(record)?;
    }
    Ok(())
}
