# 落地缓存/断点续抓

## HTML 缓存

国家页、州页、详情页抓到后按 URL 落盘（如 cache/html/<hash>.html），下次先查缓存命中则跳过请求。配合 TTL/etag 机制，定期刷新。

## 中间结果快照

州页解析后的 Vec<Mailbox>（含基础地址 + link）落成 JSON/CSV（如 cache/mailboxes_base.json），详情页补全后的结果也落一份（如 cache/mailboxes_detail.json），下次优先加载已有数据，仅对新增/缺失的 link 再抓。

## 分批断点

按州或分片保存完成标记，例如 state_done/<state>.flag 或 cache/states/<state>.json，下次跳过已完成州。

## 失败列表持久化

将失败的 link/错误原因单独记录（已有 crawl_errors.log），下次优先重试这些，再跑新增。

## 运行配置

提供“全量/增量”模式开关；增量模式只处理新增或缓存过期的项。

## 安全性

缓存和快照放在 cache/ 目录，加入 .gitignore，避免误提交；如含敏感 cookie，单独放置并注意权限。
