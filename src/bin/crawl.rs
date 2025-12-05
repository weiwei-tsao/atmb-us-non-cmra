use std::fs::File;
use std::io::{BufWriter, Write};

use atmb_us_physical::atmb::model::Mailbox;
use atmb_us_physical::atmb::{ATMBCrawl, CrawlWarning};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let crawler = ATMBCrawl::new()?;
        let (mailboxes, warnings) = crawl_and_collect(&crawler).await?;

        println!(
            "Fetched [{}] mailboxes, [{}] warnings",
            mailboxes.len(),
            warnings.len()
        );

        let out = "result/crawl_errors.log";
        if let Some(parent) = std::path::Path::new(out).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = BufWriter::new(File::create(out)?);
        for warn in warnings {
            let reason = warn.reason.replace('\n', " ");
            writeln!(w, "{} | {}", warn.link, reason)?;
        }
        w.flush()?;
        Ok(())
    })
}

async fn crawl_and_collect(
    crawler: &ATMBCrawl,
) -> color_eyre::Result<(Vec<Mailbox>, Vec<CrawlWarning>)> {
    // reuse existing fetch logic but capture errors already logged inside
    match crawler.fetch().await {
        Ok(res) => Ok((res.mailboxes, res.warnings)),
        Err(err) => Ok((
            Vec::new(),
            vec![CrawlWarning {
                name: "unknown".to_string(),
                link: "unknown".to_string(),
                reason: format!("{:?}", err),
            }],
        )),
    }
}
