import { el } from './dom';
import { describeHostState } from './lib';
import {
  STORAGE,
  type ContentMessage,
  type DetectionSummary,
  type SaveReply,
  type SelectionSummary,
  type StatusReply,
} from './messages';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

let connected = false;
let tabId: number | undefined;

/** Renders `text with \`code\`` safely as text and <code> nodes. */
function inline(target: HTMLElement, text: string): void {
  target.replaceChildren(
    ...text
      .split('`')
      .map((part, i) =>
        i % 2 ? el('code', { textContent: part }) : document.createTextNode(part),
      ),
  );
}

function setStatus(target: HTMLElement, reply: SaveReply | null, pending = false): void {
  target.className = 'item-status';
  if (pending) return void (target.textContent = 'Saving…');
  if (!reply) return void (target.textContent = '');
  target.classList.add(reply.ok ? 'success' : 'error');
  target.textContent = reply.ok ? 'Saved to GetKeys' : reply.error;
}

function toContent<T>(message: ContentMessage): Promise<T> {
  if (tabId === undefined) return Promise.reject(new Error('No active tab.'));
  return chrome.tabs.sendMessage(tabId, message) as Promise<T>;
}

async function refreshStatus(): Promise<void> {
  const pill = $('status');
  pill.className = 'pill';
  pill.textContent = 'Checking…';
  const reply: StatusReply = await chrome.runtime
    .sendMessage({ type: 'status' })
    .catch((error: unknown) => ({ state: 'error', message: String(error) }) as StatusReply);
  connected = reply.state === 'connected';
  pill.className = connected ? 'pill ok' : 'pill bad';
  pill.textContent = connected ? 'Connected' : 'Not connected';
  pill.title = reply.state === 'connected' && reply.version ? `GetKeys ${reply.version}` : '';
  $('host').hidden = connected;
  if (reply.state !== 'connected')
    inline($('host-message'), describeHostState(reply.state, reply.message));
  document.querySelectorAll<HTMLButtonElement>('[data-save]').forEach((button) => {
    button.disabled = !connected || button.dataset.saved === 'true';
  });
}

function renderDetection(detection: DetectionSummary): HTMLLIElement {
  const input = el('input', { type: 'text' });
  input.value = detection.suggestedName;
  input.maxLength = 200;
  input.setAttribute('aria-label', `Name for ${detection.providerName} key`);
  const save = el('button', { className: 'btn small', type: 'submit' });
  save.dataset.save = '';
  const status = el('p', { className: 'item-status' });
  status.setAttribute('role', 'status');
  const markSaved = () => {
    save.textContent = 'Saved';
    save.dataset.saved = 'true';
    save.disabled = true;
    input.disabled = true;
  };
  save.textContent = 'Save';
  save.disabled = !connected;
  if (detection.saved) markSaved();
  const form = el(
    'form',
    {},
    el(
      'div',
      { className: 'item-head' },
      el('span', { className: 'provider', textContent: detection.providerName }),
      el('span', { className: 'preview', textContent: detection.preview }),
    ),
    el('div', { className: 'item-row' }, input, save),
    status,
  );
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    save.disabled = true;
    setStatus(status, null, true);
    const reply = await toContent<SaveReply>({
      type: 'save-detection',
      hash: detection.hash,
      name: input.value,
    }).catch((error: unknown) => ({ ok: false as const, error: String(error) }));
    setStatus(status, reply);
    if (reply.ok) markSaved();
    else save.disabled = !connected;
  });
  return el('li', { className: 'item' }, form);
}

async function loadDetections(): Promise<boolean> {
  try {
    const { detections } = await toContent<{ detections: DetectionSummary[] }>({ type: 'list' });
    $('list').replaceChildren(...detections.map(renderDetection));
    $('empty').hidden = detections.length > 0;
    return true;
  } catch {
    $('empty').textContent = 'GetKeys can’t read this page. Reload it, or open a website.';
    $('empty').hidden = false;
    return false;
  }
}

function setupSelection(available: boolean): void {
  const start = $<HTMLButtonElement>('selection-start');
  const form = $<HTMLFormElement>('selection-form');
  const hint = $('selection-hint');
  start.disabled = !available;
  start.addEventListener('click', async () => {
    hint.hidden = true;
    const { selection } = await toContent<{ selection: SelectionSummary | null }>({
      type: 'selection',
    }).catch(() => ({ selection: null }));
    if (!selection) {
      hint.textContent = 'Select a key on the page first.';
      hint.hidden = false;
      form.hidden = true;
      return;
    }
    $('selection-provider').textContent = selection.providerName;
    $('selection-preview').textContent = selection.preview;
    $<HTMLInputElement>('selection-name').value = selection.suggestedName;
    setStatus($('selection-status'), null);
    form.hidden = false;
    start.hidden = true;
    $('selection-name').focus();
  });
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const button = form.querySelector('button')!;
    button.disabled = true;
    setStatus($('selection-status'), null, true);
    const reply = await toContent<SaveReply>({
      type: 'save-selection',
      name: $<HTMLInputElement>('selection-name').value,
    }).catch((error: unknown) => ({ ok: false as const, error: String(error) }));
    setStatus($('selection-status'), reply);
    if (reply.ok) {
      button.textContent = 'Saved';
      void loadDetections();
    } else {
      button.disabled = false;
    }
  });
}

async function setupPrompts(): Promise<void> {
  const box = $<HTMLInputElement>('prompts');
  const stored = await chrome.storage.local.get(STORAGE.prompts);
  box.checked = stored[STORAGE.prompts] !== false;
  box.addEventListener('change', () => {
    void chrome.storage.local.set({ [STORAGE.prompts]: box.checked });
  });
}

async function main(): Promise<void> {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  tabId = tab?.id;
  $('retry').addEventListener('click', () => void refreshStatus());
  void setupPrompts();
  const [, available] = await Promise.all([refreshStatus(), loadDetections()]);
  setupSelection(available);
}

void main();
