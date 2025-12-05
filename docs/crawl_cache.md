# Crawl 缓存与断点方案（草案）

目标：避免每次全量抓取，降低被封风险，并支持断点续跑。

## 缓存目录与命名
- 根目录下新增 `cache/`（加入 `.gitignore`）。
- HTML 缓存：`cache/html/<sha1(url)>.html`，可附带 `meta.json` 记录 url、状态码、抓取时间。
- 州结果快照：`cache/states/<state>.json`，存放该州解析出的 `Mailbox` 基础字段（name/price/address/link）。
- 详情结果快照：`cache/mailboxes_detail.json`（或分片），存放已补全 street 的 `Mailbox`。
- 失败列表：沿用 `result/crawl_errors.log`，并可另存 `cache/failures.json` 便于增量重试。

## 流程改造
1) **国家页/州页抓取**  
   - 先查 HTML 缓存命中则直接解析，未命中才请求并落盘。  
   - 解析州页后，将该州的基础列表写入 `cache/states/<state>.json`，成功写完可打 `cache/states/<state>.done` 标记。
2) **详情页补全**  
   - 先查 `cache/mailboxes_detail.json`（或 map<url,street>）是否已有该 link；有则直接复用。  
   - 未命中则请求详情页，落盘 HTML 缓存，解析成功后更新 detail 缓存。
3) **增量模式**  
   - 入口参数/环境变量控制：全量模式忽略缓存，增量模式仅抓未命中的州/详情。  
   - 支持按州分批：提供仅处理指定州列表的开关，便于小批调试。
4) **失败重试**  
   - 将失败 link+原因记录到 `cache/failures.json`，下次增量模式优先重试这些，再处理新增。
5) **安全与清理**  
   - `cache/` 默认不入仓库；如需重置可提供 `cargo run --bin crawl -- --clear-cache` 开关。  
   - 可在 metadata 里记录抓取时间，超过 TTL（如 7 天）视为过期重新抓取。

## 并发与节流
- 保持当前低并发与延迟，缓存命中能显著减少实际请求数，从而间接降低 503/防护概率。

## 最小可行实现（建议迭代顺序）
1) HTML 缓存 + 州快照：命中则跳过请求，写入/读取 `cache/html` 与 `cache/states`。  
2) 详情缓存：map<url, street>（可 JSON）命中直接用，未命中才抓。  
3) 增量模式开关：默认增量，提供全量/清理选项。  
4) 失败列表优先重试：加载 `cache/failures.json` 先跑失败，再跑新增。
