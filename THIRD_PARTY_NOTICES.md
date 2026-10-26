# Third-party notices

项目依赖保留各自许可证。本说明不将第三方组件重新授权为项目许可证。

- 验证码分类模型及字符索引来自 [ddddocr](https://github.com/sml2h3/ddddocr) 1.6.1，MIT；原始许可证和模型 SHA256 见 [LICENSE](src-tauri/sidecars/http-booking/xmu_http/captcha_assets/LICENSE) 与 [NOTICE](src-tauri/sidecars/http-booking/xmu_http/captcha_assets/NOTICE.md)。
- [ccl_chromium_reader](https://github.com/cclgroupltd/ccl_chromium_reader) 按 requirements.txt 固定提交获取，保留上游分发中的许可证。
- HTTP 执行器使用 Requests、ONNX Runtime、NumPy 和 Pillow；打包保留依赖分发的许可证及 ONNX Runtime 第三方声明。
- 桌面宿主和前端使用 Tauri、Rust crates、TypeScript 和 Vite。依赖版本分别记录于 Cargo.lock、package-lock.json 和执行器 requirements.txt。
- 更新说明使用 [Marked](https://github.com/markedjs/marked)（MIT）解析 Markdown，使用 [DOMPurify](https://github.com/cure53/DOMPurify)（MPL-2.0 或 Apache-2.0 双许可证）净化 HTML；版本记录于 package-lock.json，保留上游许可证声明。

厦门大学、微信及相关服务的名称和标识属于各自权利人，不表示对本项目的授权或背书。
