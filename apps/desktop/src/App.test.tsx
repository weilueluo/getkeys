import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { request } from './bridge';
import { connectedSnapshot, emptySnapshot } from '../../../tests/fixtures';
import type { Command, Snapshot } from './types';

vi.mock('./bridge', () => ({
  request: vi.fn(),
  copyCredential: vi.fn().mockResolvedValue(undefined),
}));
let snapshot: Snapshot;
beforeEach(() => {
  snapshot = emptySnapshot();
  vi.clearAllMocks();
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute('open');
  };
  vi.mocked(request).mockImplementation(async (command: Command) => {
    if (command.kind === 'snapshot') return structuredClone(snapshot);
    if (command.kind === 'resources')
      return [{ id: 'abcdefghijklmnopqrst', name: 'GetKeys', kind: 'project' }];
    if (command.kind === 'sync') return snapshot.credentials;
    if (command.kind === 'install') {
      snapshot.plugins.find((p) => p.manifest.id === command.plugin_id)!.installed = true;
      return null;
    }
    if (command.kind === 'begin_auth')
      return {
        kind: 'browser',
        flow_id: 'flow',
        url: 'https://github.com/login/device',
        user_code: 'ABCD-1234',
        interval: 5,
      };
    if (command.kind === 'reveal') return { fields: { token: 'test-secret-never-in-inventory' } };
    if (command.kind === 'import') return snapshot.credentials[0];
    throw new Error(`Unhandled test command: ${command.kind}`);
  });
});
describe('GetKeys workflows', () => {
  it('installs a plugin before starting browser authorization', async () => {
    render(<App />);
    const heading = await screen.findByRole('heading', { name: 'GitHub' });
    fireEvent.click(
      within(heading.closest('article')!).getByRole('button', { name: 'Add plugin' }),
    );
    expect(await screen.findByText('ABCD-1234')).toBeInTheDocument();
    const commands = vi.mocked(request).mock.calls.map(([c]) => c.kind);
    expect(commands.indexOf('install')).toBeLessThan(commands.indexOf('begin_auth'));
  });
  it('requests AWS configuration without assuming credentials or a default region', async () => {
    render(<App />);
    const heading = await screen.findByRole('heading', { name: 'AWS' });
    fireEvent.click(
      within(heading.closest('article')!).getByRole('button', { name: 'Add plugin' }),
    );
    expect(screen.getByLabelText('IAM Identity Center start URL')).toBeRequired();
    expect(screen.getByLabelText('SSO region')).toHaveValue('');
    expect(vi.mocked(request).mock.calls.some(([c]) => c.kind === 'begin_auth')).toBe(false);
  });
  it('keeps secret values out of inventory and hides an explicit reveal on blur', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    fireEvent.click(await screen.findByRole('button', { name: /Production API/ }));
    expect(screen.queryByText('test-secret-never-in-inventory')).not.toBeInTheDocument();
    expect(vi.mocked(request).mock.calls.some(([c]) => c.kind === 'reveal')).toBe(false);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Reveal for 30 seconds' })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Reveal for 30 seconds' }));
    expect(await screen.findByText('test-secret-never-in-inventory')).toBeInTheDocument();
    fireEvent.blur(window);
    await waitFor(() =>
      expect(screen.queryByText('test-secret-never-in-inventory')).not.toBeInTheDocument(),
    );
  });
  it('surfaces host failures without replacing them with demo data', async () => {
    vi.mocked(request).mockRejectedValue(new Error('OS credential store unavailable'));
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent('OS credential store unavailable');
    expect(screen.queryByRole('heading', { name: 'GitHub' })).not.toBeInTheDocument();
  });
  it('imports a new key without overwriting the key currently selected', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    fireEvent.click(await screen.findByRole('button', { name: /Production API/ }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Import key' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: 'Import key' }));
    expect(screen.getByLabelText('Name')).toHaveValue('');
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Replacement' } });
    fireEvent.change(screen.getByLabelText('Project API key'), {
      target: { value: 'fixture-new-key' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Validate & save' }));
    await waitFor(() =>
      expect(vi.mocked(request).mock.calls.some(([c]) => c.kind === 'import')).toBe(true),
    );
    const command = vi
      .mocked(request)
      .mock.calls.map(([c]) => c)
      .find((c) => c.kind === 'import');
    expect(command?.kind === 'import' && command.input.provider_id).toBeNull();
  });
});
