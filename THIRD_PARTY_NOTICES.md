# 第三方来源、修改及许可说明

BatterySentry 的原创部分适用根目录 `LICENSE` 中的 MIT 许可。下述第三方部分继续适用 Apache-2.0 及相关上游许可；原创部分的许可不替代第三方许可。本项目未获得 BrimDeck 官方背书，项目名称在此仅用于说明来源。

本次扩展将 BrimDeck 的接收器电量协议、型号表和相关 HID 通信流程改写为 Rust，整合进 BatterySentry。修改包括模块结构、Rust 所有权与资源释放、扫描调度、缓存、现有 ATK/AULA 路径的保留，以及界面的充电状态表达。

- 上游：[MoFeng2223/BrimDeck](https://github.com/MoFeng2223/BrimDeck)
- 固定版本：`89b93972f4c0bf57e9b8a14ceff1515b30f9c1d6`
- 来源文件：`src/BrimDeck.Core/DeviceProtocols.cs`，`src/BrimDeck/Native/Hid.cs`、`HidBatteries.cs`、`PushEvents.cs`、`CorsairReader.cs`、`GamingBatteries.cs`。
- Apache-2.0 许可全文：[licenses/BrimDeck-Apache-2.0.txt](licenses/BrimDeck-Apache-2.0.txt)。
- 上游保留的电量协议来源、作者版权和许可全文：[licenses/Receiver-protocol-notices.txt](licenses/Receiver-protocol-notices.txt)。其中对源码路径的描述指向原 BrimDeck 项目。

`src/receivers/` 是经过修改的移植代码，`gaming.rs` 对应上游 `GamingBatteries.cs`。`src/hid.rs` 的新增 Feature/Input Report 通信及标准 HID 电量读取也参考该上游实现；该文件同时保留既有通信代码。移植部分的模块结构、控制流及资源管理已适配 Rust，修改不代表移植部分成为完全独立创作。既有 ATK、AULA 驱动继续保留 BatterySentry 原有实现。

源码分发应包含根目录 `LICENSE`、本文件和 `licenses/` 下的许可文件；通过 GitHub 发布源码时，也应将这些文件提交到同一发布版本中。

编译后的程序通过 `include_str!` 嵌入根目录 MIT 许可及适用范围、本来源与修改说明、BrimDeck Apache-2.0 许可全文，以及相关第三方版权和许可全文。单独分发 exe 时，接收者可运行 `BatterySentry.exe --licenses` 查看，无需下载外部文件。若使用 ZIP 等形式发布，也应随包附带上述说明和许可文件。
