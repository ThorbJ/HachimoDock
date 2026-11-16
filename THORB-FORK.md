# HachimiDock — Thorb fork (unofficial)

**这是 Thorb 维护的非官方修改版，不是 HachimoDock 官方发布。** 官方版本请到
[ai-maker-lab/HachimoDock Releases](https://github.com/ai-maker-lab/HachimoDock/releases) 下载。
本修改版与原作者无合作、背书或官方授权关系的暗示；问题请反馈到
[ThorbJ/HachimoDock](https://github.com/ThorbJ/HachimoDock/issues)，不要提交给官方。

**This is an unofficial modified build maintained by Thorb, not an official HachimoDock release.**
Official releases: [ai-maker-lab/HachimoDock Releases](https://github.com/ai-maker-lab/HachimoDock/releases).
No affiliation with, or endorsement by, the HachimoDock author is implied. Report problems to
[ThorbJ/HachimoDock](https://github.com/ThorbJ/HachimoDock/issues), not upstream.

## 修改了什么 / What is modified

基于官方公开源码（桌面软件 0.1.201），只加了一项可选功能：**虚拟设备连接**。
在网页上运行的虚拟哈基米机可以像插上 USB 的设备一样连接本机的 HachimiDock。

- 默认关闭；在「设置 → 虚拟设备」中勾选「允许虚拟设备连接」后才开启。
- 只监听本机 `ws://127.0.0.1:23340/virtual-device`，局域网和外网连不上。
- 只接受允许列表里的网页来源（默认 `https://thorb.com`、`http://localhost:*`、`http://127.0.0.1:*`），可在设置里修改。
- USB 串口连接的行为与官方一致。

Based on the official public source (desktop 0.1.201) with one optional addition: a **virtual
device connection**. Off by default (Settings → 虚拟设备); listens on
`ws://127.0.0.1:23340/virtual-device` only; accepts only allow-listed page origins. USB serial
behaviour is unchanged. Source: branch
[`virtual-device`](https://github.com/ThorbJ/HachimoDock/tree/virtual-device) (the change proposed
upstream) and [`thorb-release`](https://github.com/ThorbJ/HachimoDock/tree/thorb-release) (this build).

注意：官方安装包的版本（如 0.1.232）比公开源码新。已装官方新版的用户装本版会回到 0.1.201 的功能；
只为试用网页虚拟设备时再装本版，真机用户建议继续用官方版。

Note: official installers (e.g. 0.1.232) are newer than the public source this build is based on.

## 安装 / Install

安装包没有购买开发者签名证书，系统会提示：

- **macOS**（Apple 芯片）：打开 dmg，把 HachimiDock 拖进「应用程序」。首次打开若提示「无法验证开发者」，
  到「系统设置 → 隐私与安全性」底部点「仍要打开」，再确认一次。
- **Windows**（x64）：运行 `-setup.exe`。若出现「Windows 已保护你的电脑」，点「更多信息 → 仍要运行」。

The installers are not signed with a paid certificate. macOS: System Settings → Privacy & Security →
"Open Anyway". Windows SmartScreen: "More info → Run anyway".

## 许可 / License

本软件按 HachimoDock 项目许可证（非商业、源码可见，见随附 `LICENSE`）再分发，保留原有版权、第三方许可
（`THIRD_PARTY_NOTICES.md`、安装包内 `licenses/`）及免责声明。HachimoDock、Pet Manager 名称与标识的权利
归原权利人所有。

Redistributed under the HachimoDock Project License (non-commercial, source-available; see the
attached `LICENSE`), with the original copyright, third-party and disclaimer notices preserved.
HachimoDock and Pet Manager names and logos belong to their owner.
