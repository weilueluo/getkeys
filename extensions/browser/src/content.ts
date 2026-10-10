import cardCss from './card.css';
import { el, keyGlyph } from './dom';
import {
  Deduper,
  MIN_SECRET_LENGTH,
  classifySelection,
  compilePatterns,
  findSecrets,
  hashValue,
  isProminent,
  maskValue,
  suggestName,
  type CompiledPattern,
  type DetectionContext,
  type SecretMatch,
} from './lib';
import {
  STORAGE,
  type ContentMessage,
  type DetectionSummary,
  type PatternsReply,
  type SaveReply,
  type SelectionSummary,
} from './messages';

interface Detection {
  hash: string;
  match: SecretMatch;
  prominent: boolean;
  saved: boolean;
  dismissed: boolean;
}

const SKIP =
  'script,style,noscript,template,textarea,[contenteditable=""],[contenteditable="true"]';
const DIALOG = 'dialog,[role="dialog"],[role="alertdialog"],[aria-modal="true"]';
const CODE = 'code,pre,kbd,samp';
const FIELD_TYPES = new Set(['text', 'password', 'search', 'url']);
const DEBOUNCE_MS = 400;
const FULL_SCAN_THRESHOLD = 200;

const deduper = new Deduper();
const detections = new Map<string, Detection>();
let patterns: CompiledPattern[] = [];
let promptsEnabled = true;
let pendingSelection: SecretMatch | null = null;
let reportedCount = -1;

function visible(element: Element): boolean {
  return typeof element.checkVisibility === 'function'
    ? element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })
    : true;
}

function pageUrl(): string {
  // Query strings and fragments can carry tokens; only the origin and path are sent.
  return /^https?:$/.test(location.protocol) ? location.origin + location.pathname : '';
}

function nameFor(match: SecretMatch): string {
  return suggestName({
    providerName: match.provider ? match.providerName : null,
    title: document.title,
    hostname: location.hostname,
  });
}

function record(match: SecretMatch, ctx: DetectionContext): Detection {
  const hash = deduper.add(match.value) ?? hashValue(match.value);
  let detection = detections.get(hash);
  if (!detection) {
    detection = { hash, match, prominent: false, saved: false, dismissed: false };
    detections.set(hash, detection);
  }
  if (!detection.prominent && isProminent(ctx)) detection.prominent = true;
  if (detection.prominent) prompt(detection);
  return detection;
}

function scanText(node: Text): void {
  if (node.data.length < MIN_SECRET_LENGTH) return;
  const parent = node.parentElement;
  if (!parent || parent === host) return;
  const found = findSecrets(node.data, patterns);
  if (!found.length || parent.closest(SKIP) || !visible(parent)) return;
  const ctx = {
    inDialog: parent.closest(DIALOG) !== null,
    inCode: parent.closest(CODE) !== null,
    readonlyField: false,
    copied: false,
  };
  for (const match of found) record(match, ctx);
}

function scanField(field: HTMLInputElement | HTMLTextAreaElement): void {
  if (field instanceof HTMLInputElement && !FIELD_TYPES.has(field.type)) return;
  if (field.value.length < MIN_SECRET_LENGTH) return;
  const found = findSecrets(field.value, patterns);
  if (!found.length || !visible(field)) return;
  const ctx = {
    inDialog: field.closest(DIALOG) !== null,
    inCode: false,
    readonlyField: field.readOnly || field.disabled,
    copied: false,
  };
  for (const match of found) record(match, ctx);
}

function scan(root: Node): void {
  if (!patterns.length) return;
  if (root instanceof Text) return scanText(root);
  if (!(root instanceof Element || root instanceof Document)) return;
  if (root instanceof HTMLInputElement || root instanceof HTMLTextAreaElement) {
    return scanField(root);
  }
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) scanText(node as Text);
  root.querySelectorAll('input, textarea').forEach((field) => {
    scanField(field as HTMLInputElement | HTMLTextAreaElement);
  });
}

function reportCount(): void {
  if (detections.size === reportedCount) return;
  reportedCount = detections.size;
  chrome.runtime.sendMessage({ type: 'detected', count: reportedCount }).catch(() => undefined);
}

function fullScan(): void {
  scan(document);
  reportCount();
}

// Debounced incremental scanning of DOM changes.
const pending = new Set<Node>();
let timer: ReturnType<typeof setTimeout> | undefined;
const observer = new MutationObserver((mutations) => {
  for (const mutation of mutations) {
    if (mutation.type === 'characterData') pending.add(mutation.target);
    else mutation.addedNodes.forEach((node) => pending.add(node));
  }
  if (!pending.size) return;
  clearTimeout(timer);
  timer = setTimeout(() => {
    const nodes = [...pending];
    pending.clear();
    if (nodes.length > FULL_SCAN_THRESHOLD) return fullScan();
    for (const node of nodes) if (node.isConnected && node !== host) scan(node);
    reportCount();
  }, DEBOUNCE_MS);
});

function selectedText(): string {
  const active = document.activeElement;
  if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {
    try {
      const { selectionStart: start, selectionEnd: end } = active;
      if (start !== null && end !== null && end > start) return active.value.slice(start, end);
    } catch {
      // Some input types do not expose a selection.
    }
  }
  return document.getSelection()?.toString() ?? '';
}

document.addEventListener(
  'copy',
  () => {
    const found = findSecrets(selectedText(), patterns);
    for (const match of found) {
      record(match, { inDialog: false, inCode: false, readonlyField: false, copied: true });
    }
    if (found.length) reportCount();
  },
  true,
);

// Floating card (shadow DOM, bottom-right).
let host: HTMLElement | null = null;
let shadow: ShadowRoot | null = null;
let current: string | null = null;
const queue: string[] = [];

function prompt(detection: Detection): void {
  if (!promptsEnabled || detection.saved || detection.dismissed) return;
  if (current === detection.hash || queue.includes(detection.hash)) return;
  queue.push(detection.hash);
  showNext();
}

function closeCard(): void {
  current = null;
  shadow?.replaceChildren();
  host?.remove();
  showNext();
}

function showNext(): void {
  if (current) return;
  let next: Detection | undefined;
  while (queue.length && !next) {
    const candidate = detections.get(queue.shift()!);
    if (candidate && !candidate.saved && !candidate.dismissed) next = candidate;
  }
  if (!next || !promptsEnabled) return;
  current = next.hash;
  renderCard(next);
}

function renderCard(detection: Detection): void {
  if (!host) {
    host = document.createElement('getkeys-capture');
    shadow = host.attachShadow({ mode: 'closed' });
  }
  const root = shadow!;
  const { match } = detection;
  const label = match.provider ? `${match.providerName} key found` : 'Key found';
  const title = el(
    'div',
    { className: 'title', textContent: label },
    el('span', { textContent: ' · Save to GetKeys' }),
  );
  title.title = `${label} · Save to GetKeys`;
  const dismissX = el('button', {
    className: 'close',
    type: 'button',
    title: 'Dismiss',
    textContent: '✕',
  });
  dismissX.setAttribute('aria-label', 'Dismiss');
  const input = el('input', { type: 'text' });
  input.id = 'name';
  input.value = nameFor(match);
  input.maxLength = 200;
  input.autocomplete = 'off';
  input.spellcheck = false;
  const nameLabel = el('label', { textContent: 'Name' });
  nameLabel.htmlFor = 'name';
  const status = el('div', { className: 'status' });
  status.setAttribute('role', 'status');
  const dismiss = el('button', {
    className: 'btn outline',
    type: 'button',
    textContent: 'Dismiss',
  });
  const save = el('button', { className: 'btn', type: 'button', textContent: 'Save' });
  const card = el(
    'div',
    { className: 'card' },
    el(
      'div',
      { className: 'head' },
      el('span', { className: 'mark' }, keyGlyph(14)),
      title,
      dismissX,
    ),
    el('div', { className: 'preview', textContent: maskValue(match.value) }),
    nameLabel,
    input,
    el('div', { className: 'row' }, status, dismiss, save),
  );
  card.setAttribute('role', 'dialog');
  card.setAttribute('aria-label', 'Save key to GetKeys');
  const style = document.createElement('style');
  style.textContent = cardCss;
  root.replaceChildren(style, card);
  if (!host.isConnected) document.documentElement.append(host);

  const onDismiss = () => {
    detection.dismissed = true;
    closeCard();
  };
  const onSave = async () => {
    save.disabled = true;
    dismiss.disabled = true;
    input.disabled = true;
    status.className = 'status';
    status.textContent = 'Saving…';
    const reply = await saveMatch(detection, input.value);
    if (current !== detection.hash) return;
    if (reply.ok) {
      status.className = 'status success';
      status.textContent = 'Saved to GetKeys';
      save.textContent = 'Saved';
      dismiss.hidden = true;
      setTimeout(() => current === detection.hash && closeCard(), 2000);
    } else {
      status.className = 'status error';
      status.textContent = reply.error;
      save.disabled = false;
      dismiss.disabled = false;
      input.disabled = false;
    }
  };
  dismiss.addEventListener('click', onDismiss);
  dismissX.addEventListener('click', onDismiss);
  save.addEventListener('click', () => void onSave());
  card.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') onDismiss();
    if (event.key === 'Enter' && event.target === input && !save.disabled) void onSave();
  });
}

async function saveMatch(detection: Detection, name: string): Promise<SaveReply> {
  let reply: SaveReply | undefined;
  try {
    reply = await chrome.runtime.sendMessage({
      type: 'save',
      provider: detection.match.provider,
      name,
      value: detection.match.value,
      url: pageUrl(),
    });
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : 'Extension unavailable.' };
  }
  if (!reply) return { ok: false, error: 'No response from GetKeys.' };
  if (reply.ok) detection.saved = true;
  return reply;
}

function summary(detection: Detection): DetectionSummary {
  return {
    hash: detection.hash,
    providerName: detection.match.provider ? detection.match.providerName : 'Key',
    preview: maskValue(detection.match.value),
    suggestedName: nameFor(detection.match),
    saved: detection.saved,
  };
}

chrome.runtime.onMessage.addListener((message: ContentMessage, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id) return false;
  switch (message?.type) {
    case 'list':
      sendResponse({ detections: [...detections.values()].map(summary) });
      return false;
    case 'save-detection': {
      const detection = detections.get(message.hash);
      if (!detection) {
        sendResponse({ ok: false, error: 'That key is no longer on this page.' });
        return false;
      }
      void saveMatch(detection, message.name).then((reply) => {
        if (reply.ok && current === detection.hash) closeCard();
        sendResponse(reply);
      });
      return true;
    }
    case 'selection': {
      const text = selectedText();
      pendingSelection = text.trim() ? classifySelection(text, patterns) : null;
      const selection: SelectionSummary | null = pendingSelection && {
        providerName: pendingSelection.providerName,
        preview: maskValue(pendingSelection.value),
        suggestedName: nameFor(pendingSelection),
      };
      sendResponse({ selection });
      return false;
    }
    case 'save-selection': {
      if (!pendingSelection) {
        sendResponse({ ok: false, error: 'Select a key on the page first.' });
        return false;
      }
      const detection = record(pendingSelection, {
        inDialog: false,
        inCode: false,
        readonlyField: false,
        copied: false,
      });
      pendingSelection = null;
      void saveMatch(detection, message.name).then((reply) => {
        reportCount();
        sendResponse(reply);
      });
      return true;
    }
    default:
      return false;
  }
});

function applyPatterns(list: unknown): void {
  patterns = compilePatterns(list);
  fullScan();
}

async function start(): Promise<void> {
  const stored = await chrome.storage.local.get([STORAGE.patterns, STORAGE.prompts]);
  promptsEnabled = stored[STORAGE.prompts] !== false;
  let list = stored[STORAGE.patterns];
  if (!Array.isArray(list) || !list.length) {
    const reply: PatternsReply | undefined = await chrome.runtime
      .sendMessage({ type: 'patterns' })
      .catch(() => undefined);
    list = reply?.patterns ?? [];
  }
  applyPatterns(list);
  observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
    characterData: true,
  });
}

chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== 'local') return;
  if (changes[STORAGE.patterns]) applyPatterns(changes[STORAGE.patterns].newValue);
  if (changes[STORAGE.prompts]) {
    promptsEnabled = changes[STORAGE.prompts].newValue !== false;
    if (!promptsEnabled && current) {
      queue.length = 0;
      closeCard();
    }
  }
});

void start();
