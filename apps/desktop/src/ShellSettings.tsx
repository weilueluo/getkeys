import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart';
import type { Controller } from './controller';

/** Launch at login and the quick-copy shortcut. GetKeys stays in the tray when its window closes. */
export function ShellSettings({ busy, perform }: Pick<Controller, 'busy' | 'perform'>) {
  const [startup, setStartup] = useState<boolean>();
  useEffect(() => {
    if (isTauri())
      isEnabled()
        .then(setStartup)
        .catch(() => {});
  }, []);
  const toggle = async () => {
    const next = !startup;
    const done = await perform(async () => {
      await (next ? enable() : disable());
      return next;
    });
    if (done !== undefined) setStartup(done);
  };
  return (
    <section className="settings-section">
      <div className="settings-label">
        <h2>Desktop</h2>
        <p>GetKeys keeps running in the tray when you close its window.</p>
      </div>
      <div className="settings-content">
        <div className="settings-row">
          <div>
            <strong id="startup-label">Start with your computer</strong>
            <small>Opens quietly in the tray when you sign in.</small>
          </div>
          <button
            type="button"
            role="switch"
            className="switch"
            aria-checked={!!startup}
            aria-labelledby="startup-label"
            disabled={busy || startup === undefined}
            onClick={() => void toggle()}
          >
            <span aria-hidden="true" />
          </button>
        </div>
        <div className="settings-row">
          <div>
            <strong>Quick copy</strong>
            <small>Find a key from any app and press Enter to copy it.</small>
          </div>
          <span className="shortcut" aria-label="Control Shift K">
            <kbd>Ctrl</kbd>
            <kbd>Shift</kbd>
            <kbd>K</kbd>
          </span>
        </div>
      </div>
    </section>
  );
}
