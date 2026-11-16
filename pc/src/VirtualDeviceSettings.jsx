/**
 * [Input] load/save_virtual_device_settings (per-computer, off by default).
 * [Output] Settings card that turns on the loopback virtual device link and edits the
 *          web page origins allowed to connect; shows listening/connected state.
 * [Pos] Optional transport settings in pc/src, rendered by ApiSettings.jsx.
 * [Sync] If this file changes, update `pc/.folder.md`.
 */
import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Card from "./shell/Card.jsx";

const originsText = (origins) => (Array.isArray(origins) ? origins : []).join("\n");

export default function VirtualDeviceSettings() {
  const [status, setStatus] = useState(null);
  const [enabled, setEnabled] = useState(false);
  const [origins, setOrigins] = useState("");
  const [busy, setBusy] = useState(true);
  const [message, setMessage] = useState("");

  const apply = (next) => {
    setStatus(next);
    setEnabled(Boolean(next?.enabled));
    setOrigins(originsText(next?.allowedOrigins));
  };

  useEffect(() => {
    let cancelled = false;
    invoke("load_virtual_device_settings").then((next) => { if (!cancelled) apply(next); })
      .catch((error) => { if (!cancelled) setMessage(`读取虚拟设备设置失败：${error}`); })
      .finally(() => { if (!cancelled) setBusy(false); });
    return () => { cancelled = true; };
  }, []);

  const save = async (nextEnabled = enabled) => {
    setBusy(true); setMessage("");
    try {
      const allowedOrigins = origins.split(/\s+/).map((value) => value.trim()).filter(Boolean);
      apply(await invoke("save_virtual_device_settings", { input: { enabled: nextEnabled, allowedOrigins } }));
      setMessage(nextEnabled ? "已开启。打开虚拟设备网页后，它会像插上 USB 的设备一样出现在设备列表。" : "已关闭，本机不再监听虚拟设备连接。");
    } catch (error) {
      setMessage(`保存失败：${error}`);
    } finally { setBusy(false); }
  };

  const connected = status?.connectedOrigin;
  const state = !status?.enabled ? "已关闭"
    : connected ? `已连接（${connected}）`
      : status?.listening ? "等待网页连接" : "未能监听";

  return (
    <Card title="虚拟设备" subtitle="让网页上运行的虚拟桌宠像插上 USB 的设备一样连接本机。默认关闭，只监听本机 127.0.0.1，只接受下方允许的网页来源。">
      <div className="api-settings__form">
        <label className="ui-field">
          <span><input type="checkbox" checked={enabled} disabled={busy}
            onChange={(event) => save(event.target.checked)} /> 允许虚拟设备连接</span>
          <span className="muted small">状态：{state}{status?.address ? ` · ${status.address}` : ""}</span>
        </label>
        <label className="ui-field" htmlFor="virtual-device-origins">
          <span className="ui-field__label">允许的网页来源（每行一个）</span>
          <textarea id="virtual-device-origins" className="ui-control" rows={4} value={origins} disabled={busy}
            spellCheck={false} onChange={(event) => setOrigins(event.target.value)} />
        </label>
        <p className="muted small">格式如 https://thorb.com；本机开发地址可写 http://localhost:*（任意端口）。其他网站无法连接。连接后设备收到的内容与 USB 设备相同，包括 Agent 状态。</p>
        <div className="llm-network-settings__actions">
          <button type="button" className="btn-primary btn-sm" disabled={busy} onClick={() => save()}>保存来源</button>
          <button type="button" className="btn-ghost btn-sm" disabled={busy}
            onClick={() => setOrigins(originsText(status?.defaultAllowedOrigins))}>恢复默认来源</button>
        </div>
      </div>
      {message && <p className="muted small" role="status">{message}</p>}
    </Card>
  );
}
