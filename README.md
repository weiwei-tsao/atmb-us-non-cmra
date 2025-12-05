# atmb-us-non-cmra

优选 [anytimemailbox](https://www.anytimemailbox.com/) 地址。

## 功能

结合 [第三方](https://www.smarty.com/) 查询接口，过滤出非 CMRA 的地址。
并按照是否为住宅地址进行排序。

运行结果保存为 csv 文件，可以在 [这里](./result/mailboxes.csv) 查看。

## 流程概览

### 第一阶段：抓取 ATMB 地址（crawl）
- 入口：`cargo run --bin crawl --release`。
- 国家页 `/l/usa` → 并发抓州页（并发 5）→ 解析地点卡片为 `Mailbox`（基础地址+详情 link）。
- 详情页补全 street（并发 3，失败重试，限速 150ms/请求），DOM 解析失败则尝试 `map_object` JSON。
- 缓存机制：
  - HTML 缓存：`cache/html/<sha1(url)>.html`，命中则不请求。
  - 基础列表：`cache/mailboxes_base.json`，存在则跳过国家/州抓取。
  - 详情缓存：`cache/mailboxes_detail.json`，命中则不请求详情，成功解析后写回。
- 失败链接与原因写入 `result/crawl_errors.log`。
- 注意：crawl bin 只负责抓取/缓存，不会生成 `mailboxes.csv`。

### 第二阶段：Smarty 校验与输出
- 入口：`cargo run --release`（主程序）。
- 读取第一阶段的 `Mailbox` 列表，调用 Smarty 校验 CMRA/RDI：
  - 环境变量 `CREDENTIALS` 配置多个 Smarty 账号，轮询使用，失败重试，达到阈值切换账号。
- 将校验结果写入 `result/mailboxes.csv`，日志见 `result/run.log`。


## 本地运行

1. 安装 [rust](https://www.rust-lang.org/) 环境，建议使用 1.80+。
2. 注册 [smarty](https://www.smarty.com/) 帐号，并获取 API key. 由于免费帐号一个月只能查询 1000 次，而 atmb 目前有 1700 多个美国地址，所以至少需要注册两个帐号
    来完成查询。
3. 设置环境变量 `CRENDENTIALS`, 值的格式为：
    `API_ID1=API_TOKEN1,API_ID2=API_TOKEN2`
    将 `API_ID1`、`API_TOKEN1` 等替换为实际的 API ID 和 TOKEN。
4. （可选）先跑抓取阶段：`cargo run --bin crawl --release`，生成/利用缓存，记录抓取告警。
5. 进入项目根目录，命令行执行 `cargo run --release`，等待程序运行完成。
6. 结果与日志：
   - `result/mailboxes.csv`：最终地址列表（含 CMRA/RDI）。
   - `result/crawl_errors.log`：抓取阶段的失败链接与原因。
   - `result/run.log`：主程序运行日志。

## TODO
使用 Github Action 定时更新地址列表
