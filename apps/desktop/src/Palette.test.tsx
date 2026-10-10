import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Palette, fuzzy } from './Palette';
import { copyCredential, request } from './bridge';
import { hidePalette } from './shell';
import { connectedSnapshot, withVercel } from '../../../tests/fixtures';

vi.mock('./bridge', () => ({ request: vi.fn(), copyCredential: vi.fn() }));
vi.mock('./shell', () => ({ hidePalette: vi.fn() }));

beforeEach(() => {
  vi.mocked(request).mockReset().mockResolvedValue(withVercel(connectedSnapshot()));
  vi.mocked(copyCredential).mockReset().mockResolvedValue(undefined);
  vi.mocked(hidePalette).mockReset().mockResolvedValue(undefined);
});
const names = () =>
  screen.queryAllByRole('option').map((o) => o.querySelector('strong')?.textContent);
const selected = () =>
  screen.getAllByRole('option').find((o) => o.getAttribute('aria-selected') === 'true');
async function open() {
  render(<Palette />);
  const search = screen.getByRole('combobox', { name: 'Search keys' });
  await waitFor(() => expect(names()).toHaveLength(3));
  return search;
}

describe('fuzzy', () => {
  it('matches letters in order and prefers runs', () => {
    expect(fuzzy('prd', 'Production API')).toBeGreaterThan(0);
    expect(fuzzy('apr', 'Production API')).toBe(-1);
    expect(fuzzy('prod', 'Production API')).toBeGreaterThan(fuzzy('prod', 'a p r o d'));
  });
});

describe('Palette', () => {
  it('lists keys that have a value to copy and filters them as you type', async () => {
    const search = await open();
    expect(search).toHaveFocus();
    // deploy-bot has no secret on this device, so there is nothing to copy.
    expect(names()).toEqual(['Production API', 'Supabase CLI', 'Vercel CLI']);
    expect(screen.getByRole('option', { name: /Production API/ })).toHaveTextContent(
      'Supabase · GetKeys',
    );
    fireEvent.change(search, { target: { value: 'vrc' } });
    expect(names()).toEqual(['Vercel CLI']);
    fireEvent.change(search, { target: { value: 'supa cli' } });
    expect(names()).toEqual(['Supabase CLI']);
    fireEvent.change(search, { target: { value: 'nothing here' } });
    expect(names()).toEqual([]);
    expect(screen.getByText('No matching keys')).toBeInTheDocument();
  });

  it('moves with the arrow keys, copies with Enter, then hides', async () => {
    const search = await open();
    expect(selected()).toHaveTextContent('Production API');
    fireEvent.keyDown(search, { key: 'ArrowUp' });
    expect(selected()).toHaveTextContent('Production API');
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(selected()).toHaveTextContent('Vercel CLI');
    fireEvent.keyDown(search, { key: 'ArrowUp' });
    expect(selected()).toHaveTextContent('Supabase CLI');
    expect(search).toHaveAttribute('aria-activedescendant', selected()!.id);
    fireEvent.keyDown(search, { key: 'Enter' });
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('Copied'));
    expect(vi.mocked(copyCredential).mock.calls).toEqual([['signin-connection', 'token']]);
    await waitFor(() => expect(hidePalette).toHaveBeenCalled(), { timeout: 2000 });
    expect(search).toHaveValue('');
  });

  it('hides on Escape or when it loses focus, without copying', async () => {
    const search = await open();
    fireEvent.change(search, { target: { value: 'vercel' } });
    fireEvent.keyDown(search, { key: 'Escape' });
    expect(hidePalette).toHaveBeenCalledTimes(1);
    expect(search).toHaveValue('');
    fireEvent.blur(window);
    expect(hidePalette).toHaveBeenCalledTimes(2);
    expect(copyCredential).not.toHaveBeenCalled();
  });

  it('reads the keys again each time it is shown', async () => {
    await open();
    fireEvent.focus(window);
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
  });
});
