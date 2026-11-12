# BatterySentry

## 功能

BatterySentry 是基于 Rust、Win32 和 Direct2D/DirectWrite 的 Windows 10/11 无线外设电量工具，可在任务栏和设备面板显示电量及充电状态，支持手动与定时刷新、历史读数缓存、开机自启、托盘图标开关、窗口缩放和设备列表滚动。

## 支持的设备

| 设备 | 接收器 VID:PID | HID 接口 |
| --- | --- | --- |
| ATK 鼠标（接收器名称为 NK Mouse） | `373B:1216` | Usage Page `FF05`，64 字节报告 |
| AULA F87 Pro 键盘 | `3554:FA09` | Usage Page `FF02`，Usage `0002`，20 字节报告 |

## 界面

![image-20261112175613899](./images/image-20261112175613899.png)

![image-20261112175624537](./images/image-20261112175624537.png)

![image-20261112175635502](./images/image-20261112175635502.png)
