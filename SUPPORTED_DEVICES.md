# 接收器电量支持

支持范围以 BrimDeck 提交 `89b93972f4c0bf57e9b8a14ceff1515b30f9c1d6` 的接收器协议为基准。以下“已接入”指请求、接口匹配和回包解析已进入实际扫描流程，不代表所有型号均经过实物验证；同品牌未列入协议表的其他产品不保证兼容。未增加蓝牙 GATT、Windows 蓝牙电量属性或 AirPods 广播读取。

| 协议或来源 | 涵盖的设备系列 | 当前状态 |
| --- | --- | --- |
| 原有 ATK BitMouse | 本机 `373B:1216`、CID/MID 型号识别；另有独立充电状态查询 | 本机读到 ATK A9 Plus NK 电量；充电回包未做实物验证 |
| 原有 AULA | `3554:FA09`，保留 `0x4A` 电量和 PSD 型号识别 | 本机读到 AULA F87 Pro 电量 |
| BitMouse、Compx、17 字节 Compx | 上游表中的 ATK、VXE、VGN、Pulsar、Hitscan、Zaopin、Scyrox、HECATE、Dareu | 已接入，新增型号未做实物验证 |
| VGN 键盘报告 | VGN `320F:5088`、ATK Z87 Pro `373B:1012` | 已接入，未做实物验证 |
| Feature Report 查询 | Endgame Gear、VAXEE、Inphic | 已接入，未做实物验证 |
| Feature 0x83 | 上游 70 个接收器/线缆记录，涉及 WLmouse、G-Wolves、LAMZU、MCHOSE A5 Pro Max | 已接入，未做实物验证 |
| Nordic 查询 | Ninjutso Sora V2 `1915:AE1C/AE11` | 已接入，未做实物验证 |
| MCHOSE 多种协议 | 原生协议、Realtek M7/L7、Gmouse G3 V2/G7 | 已接入，未做实物验证 |
| AJAZZ | AJ179 系列接收器 `249A:5C2F` 及对应线缆接口 | 已接入，支持休眠和在线状态报告；未做实物验证 |
| 独立查询 | LINGBAO M5PRO Max、NZXT Lift Elite、CHERRY PIXIU75、HECATE G5M Pro | 已接入，未做实物验证 |
| ASUS ROG/TUF | 上游型号表中的鼠标、键盘、Delta II、Pelta；Omni 接收器配对表和多设备通道 | 已接入，未做实物验证 |
| SteelSeries 鼠标 | Aerox 3/5/9、Prime/Prime Mini、Rival 3/650 | 已接入，未做实物验证 |
| Razer | 上游 84 个产品记录，包括鼠标、HyperPolling 接收器和无线键盘 | 已接入，未做实物验证 |
| Logitech HID++ 1.0/2.0 | Unifying、Bolt、Lightspeed，接收器最多六个槽位；百分比、分档、电压和耳机 ADC 电量 | 已接入，未做实物验证 |
| Corsair | Dark Core RGB Pro SE、Virtuoso SE/XT，含会话初始化、路由回退与重复读数检查 | 已接入，未做实物验证 |
| 耳机查询 | SteelSeries Arctis 7/9/Nova 7/Nova Pro；HyperX Cloud Alpha/II/Flight/III/III S/Alpha 2；MCHOSE X9 | 已接入，未做实物验证 |
| 被动上报 | Attack Shark、DELUX M900 Pro、Inphic ZHEN、二代 Rapoo、AULA G9、Steam Controller 接收器、JBL Quantum 910、PULSE 3D、Cloud Flight 2 | 已接入持续监听，未做实物验证 |
| Rapoo 一代 | VT3 PRO 接收器查询和 Input Report 状态轮询 | 已接入，未做实物验证 |
| 控制器接口 | Windows Gaming Input 提供电池报告的无线控制器，包括 Xbox 无线适配器；PlayStation 的 USB/适配器输入报告 | 已接入，未做实物验证 |
| 标准 HID 电量 | Feature Report 中的 Battery Strength/State of Charge Usage | 已接入，未做实物验证 |

## 行为说明

- 设备名称优先使用可信型号表、设备自身名称或已有型号查询。共享接收器无法确认型号时使用通用名称，不凭品牌猜型号。
- 百分比和充电状态可以同时显示；只有充电状态时显示 Charging，不虚构百分比。
- 电压曲线或分档值明确标记 approximate；这些值表示电量估算，不是电池健康度。
- 新增请求使用 Interrupt Output、Feature 或 Input Report，各自采用对应协议的已知接口。原有两台设备仍保留控制管道写入和回退逻辑。
- 被动上报设备持续保持读取句柄，只在报告改变时通知界面。首次打开时可能需要等待设备主动发送电量；JBL 等只在变化时上报的设备可能等待更久。
- 查询设备休眠或无应答时显示历史值。历史记录仅在内存中保存，接收器拔出后移除；普通查询历史最长保留 30 分钟。
- 已知的部分接收器/线缆对应关系可消除“休眠接收器 + 正在线缆设备”的重复项。不能确认关系的设备保留独立记录。
- Windows Gaming Input 的传输类型信息有限：会过滤枚举到的同 VID/PID 蓝牙接口，但无法据此严格区分所有第三方控制器的无线传输方式。

具体匹配表在 `src/receivers/catalog.rs`，其余接口限制在 `src/receivers/mod.rs`、`headsets.rs` 和 `push.rs`。协议来源和许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
