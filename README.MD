# BatterySentry

## 功能

BatterySentry 是基于 Rust、Win32 和 Direct2D/DirectWrite 的 Windows 10/11 无线外设电量工具，可在任务栏和设备面板显示电量及充电状态，支持手动与定时刷新、历史读数缓存、开机自启、托盘图标开关、窗口缩放和设备列表滚动。

## 支持的设备

| 设备 | 接收器 VID:PID | HID 接口 |
| --- | --- | --- |
| ATK 鼠标（接收器名称为 NK Mouse） | `373B:1216` | Usage Page `FF05`，64 字节报告 |
| AULA F87 Pro 键盘 | `3554:FA09` | Usage Page `FF02`，Usage `0002`，20 字节报告 |

另已接入 BrimDeck 固定版本的接收器协议，包括罗技、雷蛇、ROG、赛睿、HyperX、海盗船及多种国产外设；新增型号尚未逐台实物验证。完整范围见 [设备支持说明](SUPPORTED_DEVICES.md)，协议来源见 [第三方说明](THIRD_PARTY_NOTICES.md)。

## 界面

![image-20261112175613899](./images/image-20261112175613899.png)

![image-20261112175624537](./images/image-20261112175624537.png)

![image-20261112175635502](./images/image-20261112175635502.png)

## 许可

原创部分采用 [MIT 许可](LICENSE)；引入部分适用各自的许可，其中 BrimDeck 移植部分适用 Apache-2.0。来源、修改及完整第三方声明见 [第三方说明](THIRD_PARTY_NOTICES.md)。单文件程序已嵌入许可与来源说明，可运行 `BatterySentry.exe --licenses` 查看。
