use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};

use atmb_us_physical::atmb::{ATMBCrawl, CrawlWarning};
use atmb_us_physical::atmb::model::Mailbox;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let crawler = ATMBCrawl::new()?;
        let (mailboxes, errors) = crawl_and_collect(&crawler).await?;

        println!("Fetched [{}] mailboxes, [{}] errors", mailboxes.len(), errors.len());

        let out = "result/crawl_errors.log";
        if let Some(parent) = std::path::Path::new(out).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = BufWriter::new(File::create(out)?);
        for err in errors {
            writeln!(w, "{}", err)?;
        }
        w.flush()?;
        Ok(())
    })
}

async fn crawl_and_collect(crawler: &ATMBCrawl) -> color_eyre::Result<(Vec<Mailbox>, HashSet<String>)> {
    // reuse existing fetch logic but capture errors already logged inside
    match crawler.fetch().await {
        Ok(res) => Ok((res.mailboxes, res.warnings.into_iter().map(|w| w.link).collect())),
        Err(err) => {
            let mut set = HashSet::new();
            set.insert(format!("{:?}", err));
            Ok((Vec::new(), set))
        }
    }
}
