import * as vscode from 'vscode';
import { listKeys, runCli } from './cli';
import {
  classifyCliError,
  cliErrorMessage,
  escapeMarkdown as md,
  groupByProvider,
  type KeyInfo,
  type ProviderGroup,
} from './lib';

export type StoreState = 'loading' | 'ready' | 'empty' | 'missing' | 'error';

/** Cached `getkeys ls --json` result shared by the tree, pickers and hovers. */
export class KeyStore implements vscode.Disposable {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChange = this.changed.event;
  keys: KeyInfo[] = [];
  state: StoreState = 'loading';
  error = '';
  private loading: Promise<void> | null = null;

  reload(): Promise<void> {
    this.loading ??= this.load().finally(() => (this.loading = null));
    return this.loading;
  }

  /** Loads keys if never loaded; returns false (after telling the user) when unavailable. */
  async ensure(): Promise<boolean> {
    if (this.state !== 'ready') await this.reload();
    if (this.state === 'ready') return true;
    if (this.state === 'empty') {
      void vscode.window.showInformationMessage('No keys in GetKeys yet.');
    } else if (this.state === 'missing') {
      const choice = await vscode.window.showErrorMessage(
        'GetKeys CLI not found. Install the GetKeys app or set `getkeys.cliPath`.',
        'Set CLI Path',
      );
      if (choice) void vscode.commands.executeCommand('getkeys.configure');
    } else {
      void vscode.window.showErrorMessage(`GetKeys: ${this.error}`);
    }
    return false;
  }

  private async load(): Promise<void> {
    try {
      this.keys = await listKeys();
      this.state = this.keys.length ? 'ready' : 'empty';
      this.error = '';
    } catch (error) {
      this.keys = [];
      this.state = classifyCliError(error) === 'missing' ? 'missing' : 'error';
      this.error = cliErrorMessage(error);
    }
    await vscode.commands.executeCommand('setContext', 'getkeys.state', this.state);
    this.changed.fire();
  }

  /** Runs `getkeys refresh` (re-checks keys with their providers), then reloads. */
  async refreshRemote(): Promise<void> {
    await vscode.window.withProgress(
      { location: { viewId: 'getkeys.keys' }, title: 'Refreshing keys' },
      async () => {
        try {
          await runCli(['refresh'], 120_000);
        } catch (error) {
          void vscode.window.showErrorMessage(`GetKeys: ${cliErrorMessage(error)}`);
        }
        await this.reload();
      },
    );
  }

  dispose(): void {
    this.changed.dispose();
  }
}

export type TreeNode = { kind: 'provider'; group: ProviderGroup } | { kind: 'key'; key: KeyInfo };

export class KeysTree implements vscode.TreeDataProvider<TreeNode> {
  private readonly changed = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.changed.event;

  constructor(private readonly store: KeyStore) {
    store.onDidChange(() => this.changed.fire());
  }

  getChildren(node?: TreeNode): TreeNode[] {
    if (!node) {
      if (this.store.state !== 'ready') return [];
      return groupByProvider(this.store.keys).map((group) => ({ kind: 'provider', group }));
    }
    return node.kind === 'provider' ? node.group.keys.map((key) => ({ kind: 'key', key })) : [];
  }

  getTreeItem(node: TreeNode): vscode.TreeItem {
    if (node.kind === 'provider') {
      const { group } = node;
      const item = new vscode.TreeItem(group.provider, vscode.TreeItemCollapsibleState.Expanded);
      item.id = `provider:${group.provider}`;
      item.contextValue = 'provider';
      item.description = group.problems
        ? `${group.keys.length} · ${group.problems} need attention`
        : String(group.keys.length);
      return item;
    }
    const { key } = node;
    const item = new vscode.TreeItem(key.name, vscode.TreeItemCollapsibleState.None);
    item.id = `key:${key.id}`;
    item.contextValue = 'key';
    item.description = key.problem
      ? key.problem
      : [key.account, ...key.groups].filter(Boolean).join(' · ');
    item.iconPath = key.problem
      ? new vscode.ThemeIcon('warning', new vscode.ThemeColor('problemsWarningIcon.foreground'))
      : new vscode.ThemeIcon('key');
    const tooltip = new vscode.MarkdownString(undefined, true);
    tooltip.appendMarkdown(`**${md(key.name)}** · ${md(key.provider)}\n\n`);
    tooltip.appendCodeblock(key.ref, 'text');
    if (key.groups.length) tooltip.appendMarkdown(`\n\nGroups: ${key.groups.map(md).join(', ')}`);
    if (key.problem) tooltip.appendMarkdown(`\n\n$(warning) ${md(key.problem)}`);
    tooltip.isTrusted = false;
    item.tooltip = tooltip;
    return item;
  }

  dispose(): void {
    this.changed.dispose();
  }
}
