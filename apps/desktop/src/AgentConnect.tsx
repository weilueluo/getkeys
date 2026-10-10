import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import type { Controller } from './controller';
import type { AgentInfo, Prefs } from './types';
import { request } from './bridge';

/** Lets AI agents use keys through `getkeys mcp`, and adds it to their configuration. */
export function AgentConnect({ busy, perform }: Pick<Controller, 'busy' | 'perform'>) {
  const [prefs, setPrefs] = useState<Prefs>();
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  useEffect(() => {
    if (!isTauri()) return;
    request<Prefs>({ kind: 'get_prefs' })
      .then(setPrefs)
      .catch(() => {});
    request<AgentInfo[]>({ kind: 'agent_status' })
      .then(setAgents)
      .catch(() => {});
  }, []);
  const toggle = async () => {
    if (!prefs) return;
    const next = { ...prefs, mcp_enabled: !prefs.mcp_enabled };
    const saved = await perform(
      () => request<Prefs>({ kind: 'set_prefs', prefs: next }),
      next.mcp_enabled ? 'Agents can use your keys' : 'Agents can no longer use your keys',
    );
    if (saved) setPrefs(saved);
  };
  const connect = async (agent: AgentInfo) => {
    const status = await perform(
      () => request<AgentInfo[]>({ kind: 'connect_agent', client: agent.client }),
      `Added GetKeys to ${agent.name}. Restart it to load the change.`,
    );
    if (status) setAgents(status);
  };
  return (
    <section className="settings-section">
      <div className="settings-label">
        <h2>Connect to an agent</h2>
        <p>
          AI agents can list your keys and run commands with them. GetKeys records each value an
          agent reads.
        </p>
      </div>
      <div className="settings-content">
        <div className="settings-row">
          <div>
            <strong id="mcp-label">Allow agents</strong>
            <small>Serves your keys to agents through getkeys mcp.</small>
          </div>
          <button
            type="button"
            role="switch"
            className="switch"
            aria-checked={!!prefs?.mcp_enabled}
            aria-labelledby="mcp-label"
            disabled={busy || !prefs}
            onClick={() => void toggle()}
          >
            <span aria-hidden="true" />
          </button>
        </div>
        {agents.map((agent) => (
          <div className="settings-row" key={agent.client}>
            <div className="settings-text">
              <strong>{agent.name}</strong>
              <code className="path" title={agent.path}>
                {agent.path}
              </code>
            </div>
            <button
              className="button outline"
              disabled={busy || !prefs?.mcp_enabled}
              onClick={() => void connect(agent)}
            >
              {agent.configured ? 'Reconnect' : 'Connect'}
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}
