import {
  HOST_NAME,
  classifyNativeError,
  describeHostState,
  parsePatternsResponse,
  parsePingResponse,
  parseSaveResponse,
  validateSaveRequest,
  type ProviderPattern,
} from './lib';
import type { BackgroundMessage, PatternsReply, SaveReply, StatusReply } from './messages';
import { STORAGE } from './messages';

class HostError extends Error {
  constructor(readonly reply: Exclude<StatusReply, { state: 'connected' }>) {
    super(reply.message);
  }
}

function sendNative(message: object): Promise<unknown> {
  return new Promise((resolve, reject) => {
    try {
      chrome.runtime.sendNativeMessage(HOST_NAME, message, (response: unknown) => {
        const error = chrome.runtime.lastError;
        if (!error) return resolve(response);
        const state = classifyNativeError(error.message ?? '');
        reject(new HostError({ state, message: describeHostState(state, error.message) }));
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      reject(new HostError({ state: 'error', message }));
    }
  });
}

function failure(error: unknown): Exclude<StatusReply, { state: 'connected' }> {
  if (error instanceof HostError) return error.reply;
  return { state: 'error', message: error instanceof Error ? error.message : String(error) };
}

async function status(): Promise<StatusReply> {
  try {
    const ping = parsePingResponse(await sendNative({ type: 'ping' }));
    if (!ping.ok) return { state: 'error', message: ping.error };
    void refreshPatterns().catch(() => undefined);
    return { state: 'connected', version: ping.value };
  } catch (error) {
    return failure(error);
  }
}

async function refreshPatterns(): Promise<ProviderPattern[]> {
  const parsed = parsePatternsResponse(await sendNative({ type: 'patterns' }));
  if (!parsed.ok) throw new HostError({ state: 'error', message: parsed.error });
  await chrome.storage.local.set({ [STORAGE.patterns]: parsed.value });
  return parsed.value;
}

async function patterns(): Promise<PatternsReply> {
  const stored = await chrome.storage.local.get(STORAGE.patterns);
  const cached = stored[STORAGE.patterns];
  if (Array.isArray(cached) && cached.length) return { patterns: cached as ProviderPattern[] };
  try {
    return { patterns: await refreshPatterns() };
  } catch {
    return { patterns: [] };
  }
}

async function save(message: unknown): Promise<SaveReply> {
  const request = validateSaveRequest(message);
  if (!request.ok) return request;
  try {
    const saved = parseSaveResponse(await sendNative(request.value));
    return saved.ok ? { ok: true, id: saved.value } : saved;
  } catch (error) {
    return { ok: false, error: failure(error).message };
  }
}

async function setBadge(tabId: number, count: number): Promise<void> {
  await chrome.action.setBadgeBackgroundColor({ tabId, color: '#ea580c' });
  await chrome.action.setBadgeText({ tabId, text: count > 0 ? String(Math.min(count, 99)) : '' });
}

chrome.runtime.onInstalled.addListener(() => void refreshPatterns().catch(() => undefined));
chrome.runtime.onStartup.addListener(() => void refreshPatterns().catch(() => undefined));

chrome.runtime.onMessage.addListener((raw: BackgroundMessage, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id) return false;
  const reply = (promise: Promise<unknown>) => {
    promise.then(sendResponse, (error: unknown) => sendResponse(failure(error)));
    return true;
  };
  switch (raw?.type) {
    case 'status':
      return reply(status());
    case 'patterns':
      return reply(patterns());
    case 'save':
      return reply(save(raw));
    case 'detected':
      if (sender.tab?.id !== undefined && typeof raw.count === 'number') {
        void setBadge(sender.tab.id, raw.count).catch(() => undefined);
      }
      return false;
    default:
      return false;
  }
});
