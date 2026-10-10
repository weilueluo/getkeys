import type { HostState, ProviderPattern } from './lib';

export type StatusReply =
  | { state: 'connected'; version: string }
  | { state: Exclude<HostState, 'connected'>; message: string };

export type SaveReply = { ok: true; id: string } | { ok: false; error: string };

export interface DetectionSummary {
  hash: string;
  providerName: string;
  preview: string;
  suggestedName: string;
  saved: boolean;
}

export interface SelectionSummary {
  providerName: string;
  preview: string;
  suggestedName: string;
}

/** Messages handled by the background worker. */
export type BackgroundMessage =
  | { type: 'status' }
  | { type: 'patterns' }
  | { type: 'save'; provider: string | null; name: string; value: string; url: string }
  | { type: 'detected'; count: number };

/** Messages handled by the content script (sent by the popup). */
export type ContentMessage =
  | { type: 'list' }
  | { type: 'save-detection'; hash: string; name: string }
  | { type: 'selection' }
  | { type: 'save-selection'; name: string };

export interface PatternsReply {
  patterns: ProviderPattern[];
}

export const STORAGE = {
  patterns: 'patterns',
  prompts: 'prompts',
} as const;
