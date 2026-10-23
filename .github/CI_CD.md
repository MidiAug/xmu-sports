# CI/CD

## 自动检查

`checks.yml` 在 push、PR 和手动触发时运行 Windows 检查：公开文件边界、前端测试与构建、HTTP 执行器离线测试、Rust 宿主单元测试。依赖按 lockfile、requirements 和固定 Action 提交安装。检查不调用学校登录或预约接口，不制作安装包。

浏览器交互、微信 UI 和安装升级不在自动检查中，需按 CONTRIBUTING 单独验证。

## 手动安装包与草稿

在 Actions 的 **Windows package** 中选择分支并手动运行。默认仅上传安装包和 SHA256 为临时构建产物，不改变版本号，不发布 Release。

填写 `release_tag`（例如 `v0.1.26`）后，流程检查当前分支为 main、更新记录含该版本、应用版本一致、该标签和 Release 尚不存在，再构建并创建 **draft Release**。安装包未经代码签名；人工确认来源、功能和 Windows 行为后再在 GitHub 发布草稿。现有标签不复用，避免将不同源代码绑定到同一版本。

工作流不在 push 时自动打包。应用“关于”页支持手动检查与下载正式 Release，通过资产大小和 SHA256SUMS.txt 校验安装包，不自动运行安装器。发布时须同时保留 `XMU Sports_X.Y.Z_x64-setup.exe` 与 `SHA256SUMS.txt`；草稿、预发布与源码归档不会被作为更新。没有正式 Release 时显示“暂未发布正式版本”。

无需配置学校凭证；不得向 Actions 上传 Cookie、个人数据库或微信会话。仓库内置 GITHUB_TOKEN 仅在手动打包任务授予 contents:write，其余检查为只读。构建产物保留 14 天，真实用户数据不参与构建。
