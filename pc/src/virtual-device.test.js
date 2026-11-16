/**
 * [Input] Read VirtualDeviceSettings.jsx, ApiSettings.jsx, DeviceStatusBar.jsx and the Rust command wiring.
 * [Output] Static Node coverage that the optional virtual device link is reachable from Settings,
 *          off by default, loopback-only, and shown as a connected device without a separate device path.
 * [Pos] test node in pc/src
 * [Sync] If this file changes, update `pc/.folder.md`.
 */
import { readFileSync } from "node:fs";
import { test } from "node:test";
import assert from "node:assert/strict";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

test("Settings exposes the virtual device card through the saved-settings commands", () => {
  const ui = read("./VirtualDeviceSettings.jsx");
  const lib = read("../src-tauri/src/lib.rs");
  assert.match(read("./ApiSettings.jsx"), /<VirtualDeviceSettings \/>/);
  for (const command of ["load_virtual_device_settings", "save_virtual_device_settings"]) {
    assert.ok(ui.includes(command));
    assert.ok(lib.includes(`virtual_device_settings::${command}`));
  }
  assert.match(ui, /默认关闭/);
  assert.match(ui, /127\.0\.0\.1/);
  assert.match(ui, /allowedOrigins/);
});

test("virtual device link is opt-in, loopback-only and reuses the serial device path", () => {
  const settings = read("../src-tauri/src/virtual_device_settings.rs");
  const transport = read("../src-tauri/src/usb_serial/virtual_device.rs");
  assert.match(settings, /Self \{ enabled: false,/);
  assert.match(transport, /TcpListener::bind\(\(Ipv4Addr::LOCALHOST, config\.port\)\)/);
  assert.doesNotMatch(transport, /0\.0\.0\.0|Ipv4Addr::UNSPECIFIED/);
  assert.match(transport, /StatusCode::FORBIDDEN/);
  // Listed by usb_scan_devices and probed by the same connect path; no new frontend device API.
  assert.match(read("../src-tauri/src/usb_serial.rs"), /devices\.extend\(self\.virtual_devices\.devices\(\)\)/);
});

test("a connected virtual device is labelled in the device status bar", () => {
  const bar = read("./dashboard/DeviceStatusBar.jsx");
  assert.match(bar, /startsWith\("virtual-device-"\)/);
  assert.match(bar, /"虚拟设备"/);
});
