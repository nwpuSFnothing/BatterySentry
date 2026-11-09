# BatterySentry

适用于 Windows 10/11 的 2.4 GHz 无线鼠标、键盘电量监控工具，使用 Rust 和原生 Win32 界面开发。当前版本：**0.2.3**。

程序在任务栏空闲区域显示设备名称和电量，支持手动刷新、定时刷新和开机自启。运行时无需 Python 或浏览器引擎，不访问网络，不生成日志文件。

```text
ATK A9 Plus NK      83%
AULA Device       ~65%
```

> 上方为显示格式示例。`~` 表示上一次成功读取的结果，不代表当前实时电量。

## 功能

- 在主显示器的水平任务栏内显示鼠标、键盘电量；空间不足时自动隐藏，避免遮挡其他任务栏项目。
- 原生无边框设置面板，支持 DPI 缩放和任务栏明暗主题适配。
- 刷新间隔可选 30 秒、1 分钟、2 分钟、5 分钟、10 分钟、30 分钟，默认 1 分钟。
- 不同设备驱动并行读取，设置面板与任务栏共用读取结果。
- 支持静默开机自启和单实例运行。
- `Tray icon` 开关控制通知区域图标，默认开启并自动保存。单击或双击图标打开主界面，右键可打开界面、刷新或退出；关闭图标不影响任务栏电量文字。
- 设置面板默认单列显示 3 个设备，支持虚拟滚动、滚动条和拖动边框调整大小；名称左对齐、电量右对齐，拉宽窗口可显示更长名称。
- 最小化与关闭按钮位于右上角，关闭悬停显示红色方形背景；其他按钮和开关提供短时过渡动画，尊重系统动画设置。
- 刷新间隔使用自绘圆角选项卡，支持鼠标及键盘操作。窗口尺寸会自动保存，刷新不会改变用户设定的尺寸。
- 型号通过设备身份查询与已知资料匹配；无法确认型号时显示“品牌 + Device”，例如 `AULA Device`，不会根据接收器 PID 猜测型号。
- 仅向已匹配的接收器发送电量查询命令，不对未知设备发送探测命令。

## 支持的设备

| 设备 | 接收器 VID:PID | HID 接口 |
| --- | --- | --- |
| ATK 鼠标（接收器名称为 NK Mouse） | `373B:1216` | Usage Page `FF05`，64 字节报告 |
| AULA F87 Pro 键盘 | `3554:FA09` | Usage Page `FF02`，Usage `0002`，20 字节报告 |

支持范围以接收器标识和协议为准，不代表支持所有 ATK 或 AULA 型号。已有项目记录中，两款本地接收器均成功返回过电量；其他固件或接收器版本仍需验证。

设备列表只显示当前枚举到且匹配受支持协议的接口，无设备时显示空状态。型号识别与电量查询互不依赖；ATK 使用 CID/MID，AULA 使用 PSD。身份查询失败不会丢弃已读到的电量。本机实测 `(CID=2, MID=4)` 显示 `ATK A9 Plus NK`；无线 PSD `03 00 00 00 01 0B` 按用户选定的有线型号对照显示 `AULA F87 Pro`，不根据接收器 PID 猜型号。

任务栏按稳定的设备顺序显示前两台设备；更多设备用 `(+N)` 提示，打开面板查看全部。

设备休眠或离线时可能不响应，唤醒后可手动刷新。AULA 查询最多尝试 3 次，每次等待 1 秒。识别为有线供电时显示 `Charging`，不推测百分比。

读取超时后，程序最多在内存中保留 30 分钟的历史读数，并加上 `~` 标记；接收器从设备列表消失后，对应缓存会清除。电量缓存不写入磁盘。

## 快速使用

本仓库只保存源码，不包含编译后的 EXE。先按下文完成构建，再运行 `target\release\BatterySentry.exe`；也可以将这个 EXE 复制到固定目录单独运行。

| 操作 | 效果 |
| --- | --- |
| 双击启动 EXE | 显示设置面板，并启动任务栏电量显示 |
| 左键点击任务栏电量 | 立即刷新 |
| 右键点击或双击任务栏电量 | 打开设置面板 |
| 关闭面板或按 Escape | 关闭面板，程序继续运行 |
| 点击面板中的 `Exit` | 完全退出程序 |

电量文字显示位于任务栏内，通知区域图标作为可选的主界面入口。任务栏拥挤或前台应用全屏时，电量文字可能隐藏；布局恢复后会重新计算位置。再次运行 EXE 可打开已有实例的设置面板，即使通知区域图标已关闭也可使用。

### 开机自启与配置

在设置面板中开启 `Launch at sign-in` 后，程序会在 Windows 启动文件夹创建指向当前 EXE 的快捷方式，并使用 `--background` 参数静默启动。

请先把 EXE 放到长期使用的位置，再开启自启。移动 EXE 后，需要关闭再重新开启自启，以更新快捷方式。普通启动不会自动修改自启设置。

刷新间隔保存在：

```text
%LOCALAPPDATA%\BatterySentry\settings.ini
```

配置同时保存 `window_width`、`window_height`，单位为逻辑像素。边框拖拽缩放改变可用空间，不放大字体。面板支持滚轮、滚动条、方向键、Page Up/Down、Home/End；Tab 切换按钮，Enter/空格激活，Escape 先关闭选项卡、再关闭面板。

`show_tray_icon=1` 表示显示通知区域图标，`0` 表示隐藏。旧配置没有该项时默认显示。单实例按 Windows 窗口站和桌面隔离，开发工具的隔离桌面不会阻止普通桌面启动；重复运行 `--background` 保持静默，普通双击明确打开面板。

## 从源码构建

### 环境要求

- Windows 10/11；当前构建脚本使用 Windows SDK 中的 x64 资源编译器。
- 支持 Rust 2024 edition 的 Rust 工具链，使用 MSVC 目标。
- Visual Studio Build Tools，安装“使用 C++ 的桌面开发”及 Windows SDK，提供 MSVC 链接器和 `rc.exe`。

在项目根目录打开 PowerShell 执行：

```powershell
cargo build --release --locked
.\target\release\BatterySentry.exe
```

首次构建需要获取 Cargo 依赖。`build.rs` 会将 `assets/BatterySentry.ico` 和 `assets/app.rc` 编译进 EXE，运行时无需随附图标文件。

如需在项目根目录放置可执行文件：

```powershell
Copy-Item .\target\release\BatterySentry.exe .\BatterySentry.exe
.\BatterySentry.exe --background
```

更新或覆盖 EXE 前，先退出正在运行的实例。`Cargo.lock` 随源码提交，便于固定依赖版本；构建输出和复制出的 EXE 已由 `.gitignore` 排除。

### 命令行参数

以下示例假定已将 EXE 复制到项目根目录：

| 命令 | 用途 |
| --- | --- |
| `.\BatterySentry.exe` | 正常启动，打开设置面板 |
| `.\BatterySentry.exe --background` | 后台启动，仅显示任务栏电量 |
| `.\BatterySentry.exe --quit` | 请求正在运行的实例退出 |
| `.\BatterySentry.exe --once` | 读取一次电量并输出，然后退出 |
| `.\BatterySentry.exe --list` | 列出 HID 接口及接收器信息 |
| `.\BatterySentry.exe --help` | 显示帮助 |

使用 `--once` 做独立 HID 诊断前，建议先用 `--quit` 退出后台实例，避免同时访问设备。

## 开发与验证

```powershell
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --release --locked
```

项目包含协议解码、设备匹配、缓存、配置、启动快捷方式和任务栏布局等测试。Windows 下可运行开发辅助脚本：

```powershell
python tools/taskbar_smoke.py
```

该脚本使用模拟任务栏和独立进程的 LiteMonitor 形状窗口，检查布局、窗口焦点切换和程序退出。Python 只用于开发检查，运行 BatterySentry 不需要 Python。截图等检查产物写入 `artifacts/`，不会提交到仓库。

模拟任务栏检查不能替代真实 Explorer 桌面验证。`tools/smoke.py` 包含旧版通知区域检查及截图辅助代码，当前版本应使用 `tools/taskbar_smoke.py`。目前没有长期运行资源占用的测量结论。

## 项目结构

```text
BatterySentry/
├── assets/                 # 图标源文件、ICO 和 Windows 资源定义
├── docs/
│   ├── PROTOCOL.md         # 接收器协议与参考资料
│   └── TASKBAR.md          # 任务栏布局方案和验证限制
├── src/
│   ├── main.rs             # 入口与命令行参数
│   ├── panel.rs            # 可缩放布局、虚拟列表与交互状态
│   ├── app.rs              # 原生界面与应用生命周期
│   ├── config.rs           # 刷新间隔配置
│   ├── drivers.rs          # 设备驱动注册、扫描与缓存
│   ├── hid.rs              # Windows HID 传输
│   ├── protocol.rs         # 协议请求与响应解析
│   ├── startup.rs          # 开机自启快捷方式
│   ├── taskbar.rs          # 任务栏显示与布局
│   ├── tray.rs             # 可选通知区域图标与右键菜单
│   └── win.rs              # Win32 辅助函数
├── tools/                  # 开发检查脚本
├── build.rs                # Windows 资源编译
├── Cargo.toml
├── Cargo.lock
├── PORTING.md              # 移植与设计记录
└── README.MD
```

新增设备时，在 `src/drivers.rs` 中实现 `DeviceDriver` 并加入 `registry()`，在 `src/protocol.rs` 中添加经过验证的协议编解码。请精确匹配接收器 VID/PID、Usage 和报告长度，并为响应校验添加测试。

型号识别的命令、来源及当前验证限制见 [协议说明](docs/PROTOCOL.md)，本次界面行为见 [界面说明](docs/PANEL.md)。

## 已知限制与排查

- **任务栏没有电量文字**：目前面向主显示器的水平任务栏；不支持垂直任务栏。空间不足、全屏应用或无法可靠识别空闲区域时会隐藏，可打开设置面板查看状态。
- **电量显示 `--` 或 `~`**：确认接收器已连接且设备已唤醒，再刷新；同时检查接收器标识是否在支持列表中。
- **提示找不到 `rc.exe` 或链接器**：检查 Visual Studio C++ Build Tools 和 Windows SDK 是否安装完整，必要时在 Visual Studio 开发者 PowerShell 中构建。
- **与其他任务栏组件的兼容性**：布局依赖 Explorer 窗口和 UI Automation 信息，实际效果可能随 Windows 版本或第三方组件变化。

更多细节见 [任务栏实现说明](docs/TASKBAR.md)、[接收器协议说明](docs/PROTOCOL.md) 和 [移植记录](PORTING.md)。
