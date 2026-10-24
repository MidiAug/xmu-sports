# 厦大体育 HTTP 执行器（0.1.20）

当前桌面版只保留 HTTP 预约。登录恢复由宿主 Rust 代码调用 Windows UI Automation 完成：正常唤起厦大体育、查找登录入口、选择协议、调用授权按钮、进入场馆。界面无可用元素、需要扫码或新增身份认证时提示本人处理，不加载视觉兜底，不附加内核或开放调试端口。

进入官方 H5 后，worker 从本机微信 WebView Local Storage 读取 xdty.xmu.edu.cn 的 hrefstr/isNewAjaxRes。有效票据通过学校 checkLogin 换取 worker 自己的 PHPSESSID，再读取 mySubscribe 核验账号。账号指纹必须一致；仅缓存可读或页面已打开不能代表认证成功。没有可用票据时，不能靠自行构造 HTTP 参数替代微信/学校签发。

worker 的 check 动作只运行 checkLogin 和 mySubscribe；check_existing 额外读取 subDetails 以核对指定既有预约，均不获取验证码或调用 addOrder。宿主 --login-check-only / --login-ui-check-only 在调度器初始化前运行，用于安全地测试登录与读取已有预约；不启动完整应用测试，以免恢复已保存的定时预约。

0.1.16 提交超时、断网或响应不明后，按 1、2、4、8、15 秒间隔自动等待并查询列表与详情。查单使用当次实际选择的场地与完整时段；确认后回写原请求记录并返回成功。五轮仍不明确或被停止时保留结果待确认，绝不据“没查到”自动重发，也不切换备选绕过去重。详情取消状态会被排除。

验证码识别保留旧版实际使用的 common_old.onnx（13,606,051 字节），使用 ONNX Runtime CPU、Pillow 和 NumPy 完成等价预处理与 CTC 解码。仅输出英文字母和数字，不包含页面 OCR、检测模型、滑块引擎、OpenCV、完整 ddddocr、Frida 或 websocket 调试组件。模型与字符索引来源及许可证见 xmu_http/captcha_assets/NOTICE.md。

实际 HTTP 预约保留验证码、实时选场、提交前校验、原账号约束和结果查单；网络结果未知时保留去重记录，不盲目重提。去重记录位于 %LOCALAPPDATA%/xmu-sports/http-booking/requests/，不保存 Cookie、token 或验证码。登录与只读诊断不经过提交路径。

未发布修复：仅 getInterval 明确返回整数 status=0 且 info 为“参数错误”时，原 HTTP 会话等待 2 秒后使用相同参数重试一次。每个 Client 会话只有一次恢复额度，等待中可停止；第二次失败保留原错误。两次请求各自保留脱敏诊断，运行与历史显示等待阶段。该规则不应用于登录、预检、配置、验证码或 addOrder，不重跑预约；提交结果未知仍只读查单，不重复提交。该容错不代表已确定学校服务端拒绝的根因。

构建：在仓库根目录运行 npm run tauri:build。终端用户无需安装 Python。依赖安装与冻结由 scripts/ensure_http_bundle.ps1 管理；替换资源目录以避免旧 DLL 残留。旧 runtime_entry/runtime_probe/runtime_wire 与 auth-hook 资料仅作历史研究，不包含在冻结 worker 中。

开发测试：

```powershell
.venv/Scripts/python.exe -m pytest -q
```

在仓库根目录审计实际冻结模块与资源：

```powershell
src-tauri/sidecars/http-booking/.venv/Scripts/python.exe scripts/test_lite_bundle.py
```

本次轻量版验证范围为自动登录恢复、已有预约读取、离线验证码回归和安装包构建，不提交、取消或修改预约。历史版本的实测行为与内核实验见 docs/research 中带日期的记录。

0.1.17 减少一次重复登录，将未确认提交检查放在同一会话；拒绝未知 action，新增脱敏阶段/API 耗时指标，查单展示轮次和等待时间。

0.1.18 新增基于原 journal 的只读再次查单，以及定时任务专用的持续 worker 与本地模型推理预热。持续进程绑定冻结任务和账号，处理一次预约即退出；诊断模式禁止预约。

0.1.19 补上正式执行初始认证失败的恢复分支。仅完整回复中的积极未提交证明可授权宿主恢复原账号，再用新进程、原冻结任务和原去重标识继续一次。已消费进程不重置，响应不明不重放；新增拒绝接口的固定别名日志。

0.1.20 将宿主定时准备与模型预热改到开抢前一分钟，保留前十五秒复查。
