# Crawl 功能与现状（2025-12-05）

## 功能与流程（当前 crawl bin 行为）
- 入口：`cargo run --bin crawl --release`，调用 `ATMBCrawl::fetch()`。
- 过程概览：
  1) 国家页 `/l/usa`：用 DOM 选择器抓取州链接/州名（`CountryPage::parse_html`）。
  2) 州页并发抓取：并发上限 5（`buffer_unordered(5)`），`StatePage::parse_html` 用 scraper 解析地点卡片，拿到 name/price/line1(line2)/plan link。
  3) 转 Mailbox：`StatePage::to_mailboxes` 生成基础地址的 `Mailbox`（detail link 带着）。
  4) 详情页补全 street：并发上限 3，每请求后 sleep 150ms，失败重试 3 次。`LocationDetailPage::parse_html` 先 DOM（`LOCATION_DETAIL_SELECTOR`），缺失则从 `map_object` JSON 的 `foraddss` 解析。
- 输出：**crawl bin 仅写 `result/crawl_errors.log`**（link | reason）。它不会产出 `mailboxes.csv`；那是主程序 `cargo run --release` 做的。
- 请求头：真实 UA + Accept/Language/Connection/Cache-Control + `Accept-Encoding: identity`（避免压缩体被误解析）。`reqwest` 开启了 gzip/br feature。

## 已知问题
- 详情页频繁返回 503（防护页 “Your access to this site has been limited by the site owner”），导致大量失败。
- 为避封降低了并发和加了延迟，整体抓取时间显著变长。
- 当站点返回防护页或压缩体未正确解压时，会解析不到结构，错误写入 `crawl_errors.log`。

## 后续可选优化方向
1) 进一步降速：并发 1–2，随机更长 sleep，分批按州抓取，降低瞬时请求量。  
2) Cookie/会话复用：从浏览器获取 cookie（若含放行标记），在请求中复用。  
3) 代理/出口轮询：多 IP 轮询以分散限流。  
4) 更完整的浏览器指纹或 headless 浏览器：成本较高，仅在必要时考虑。  
5) 增量抓取/缓存：记录成功的详情页，后续只抓新增或过期链接，减少总请求量。
