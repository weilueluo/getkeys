import * as vscode from 'vscode';
import { cliPath, runCli, workspaceCwd } from './cli';
import {
  buildEnvArgs,
  buildGetArgs,
  buildRunArgs,
  cleanValue,
  cliErrorMessage,
  collectGroups,
  defaultField,
  describeRef,
  findRefAt,
  refForField,
  type KeyField,
  type KeyInfo,
  type Selection,
} from './lib';
import { KeyStore, KeysTree, type TreeNode } from './store';

const TASK_TYPE = 'getkeys';

interface GetKeysTaskDefinition extends vscode.TaskDefinition {
  command: string;
  groups?: string[];
  keys?: string[];
}

interface KeyPick extends vscode.QuickPickItem {
  key: KeyInfo;
}

interface FieldPick extends vscode.QuickPickItem {
  field: KeyField | null;
}

interface SelectionPick extends vscode.QuickPickItem {
  group?: string;
  ref?: string;
}

let clipboardTimer: ReturnType<typeof setTimeout> | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const store = new KeyStore();
  const tree = new KeysTree(store);
  const view = vscode.window.createTreeView('getkeys.keys', {
    treeDataProvider: tree,
    showCollapseAll: true,
  });
  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
  status.name = 'GetKeys';
  status.text = '$(key) GetKeys CLI not found';
  status.tooltip = 'Install the GetKeys app or set getkeys.cliPath';
  status.command = 'getkeys.configure';
  status.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');

  store.onDidChange(() => {
    const problems = store.keys.filter((key) => key.problem).length;
    view.badge = problems
      ? { value: problems, tooltip: `${problems} key${problems === 1 ? '' : 's'} need attention` }
      : undefined;
    if (store.state === 'missing') status.show();
    else status.hide();
  });

  const pickKey = async (title: string): Promise<KeyInfo | undefined> => {
    if (!(await store.ensure())) return undefined;
    const picked = await vscode.window.showQuickPick<KeyPick>(
      store.keys.map((key) => ({
        key,
        label: key.problem ? `$(warning) ${key.name}` : key.name,
        description: key.provider,
        detail: [key.account, ...key.groups].filter(Boolean).join(' · ') || undefined,
      })),
      { title, placeHolder: 'Search keys', matchOnDescription: true, matchOnDetail: true },
    );
    return picked?.key;
  };

  const pickField = async (key: KeyInfo, title: string, allowDefault: boolean) => {
    if (key.fields.length <= 1) return { field: key.fields[0] ?? null };
    const items: FieldPick[] = key.fields.map((field) => ({
      field,
      label: field.label,
      description: field.secret ? 'secret' : undefined,
    }));
    if (allowDefault) items.unshift({ field: null, label: 'Key', description: 'default field' });
    return vscode.window.showQuickPick(items, {
      title: `${title} · ${key.name}`,
      placeHolder: 'Field',
    });
  };

  const keyFrom = (node?: TreeNode) => (node?.kind === 'key' ? node.key : undefined);

  const insertRef = async (node?: TreeNode) => {
    const key = keyFrom(node) ?? (await pickKey('Insert Key Reference'));
    if (!key) return;
    const picked = await pickField(key, 'Insert Key Reference', true);
    if (!picked) return;
    const ref = refForField(key, picked.field?.key ?? null);
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
      await vscode.env.clipboard.writeText(ref);
      void vscode.window.showInformationMessage(`No open editor — copied ${ref}`);
      return;
    }
    await editor.edit((edit) => {
      for (const selection of editor.selections) edit.replace(selection, ref);
    });
  };

  const copyKey = async (node?: TreeNode) => {
    const key = keyFrom(node) ?? (await pickKey('Copy Key'));
    if (!key) return;
    const picked =
      key.fields.length > 1
        ? await pickField(key, 'Copy Key', false)
        : { field: defaultField(key) };
    if (!picked) return;
    const field = picked.field;
    let value: string;
    try {
      value = cleanValue(await runCli(buildGetArgs(key.ref, field?.key ?? null)));
    } catch (error) {
      void vscode.window.showErrorMessage(`GetKeys: ${cliErrorMessage(error)}`);
      return;
    }
    await vscode.env.clipboard.writeText(value);
    const seconds = vscode.workspace
      .getConfiguration('getkeys')
      .get<number>('clipboardClearSeconds', 45);
    clearTimeout(clipboardTimer);
    if (seconds > 0) {
      clipboardTimer = setTimeout(async () => {
        if ((await vscode.env.clipboard.readText()) === value)
          await vscode.env.clipboard.writeText('');
      }, seconds * 1000);
    }
    const what = field && key.fields.length > 1 ? `${key.name} (${field.label})` : key.name;
    void vscode.window.setStatusBarMessage(
      seconds > 0 ? `$(check) Copied ${what} · clears in ${seconds}s` : `$(check) Copied ${what}`,
      4000,
    );
  };

  const pickSelection = async (title: string): Promise<Selection | undefined> => {
    if (!(await store.ensure())) return undefined;
    const last = context.workspaceState.get<Selection>('getkeys.selection', {
      groups: [],
      keys: [],
    });
    const items: SelectionPick[] = [
      ...collectGroups(store.keys).map((group) => ({
        group,
        label: `$(folder) ${group}`,
        description: 'group',
        picked: last.groups.includes(group),
      })),
      ...store.keys.map((key) => ({
        ref: key.ref,
        label: `$(key) ${key.name}`,
        description: key.provider,
        picked: last.keys.includes(key.ref),
      })),
    ];
    const picked = await vscode.window.showQuickPick(items, {
      title,
      placeHolder: 'Choose groups or keys',
      canPickMany: true,
      matchOnDescription: true,
    });
    if (!picked?.length) return undefined;
    const selection: Selection = {
      groups: picked.flatMap((item) => (item.group ? [item.group] : [])),
      keys: picked.flatMap((item) => (item.ref ? [item.ref] : [])),
    };
    await context.workspaceState.update('getkeys.selection', selection);
    return selection;
  };

  const writeEnv = async () => {
    const selection = await pickSelection('Write .env');
    if (!selection) return;
    const folder = vscode.workspace.workspaceFolders?.[0]?.uri;
    const target = await vscode.window.showSaveDialog({
      title: 'Write .env',
      saveLabel: 'Write',
      defaultUri: folder ? vscode.Uri.joinPath(folder, '.env') : undefined,
    });
    if (!target) return;
    try {
      await runCli(buildEnvArgs(selection, target.fsPath));
    } catch (error) {
      void vscode.window.showErrorMessage(`GetKeys: ${cliErrorMessage(error)}`);
      return;
    }
    const open = await vscode.window.showInformationMessage(
      `Wrote ${vscode.workspace.asRelativePath(target)}. Keep it out of version control.`,
      'Open',
    );
    if (open) await vscode.window.showTextDocument(target);
  };

  const createTask = (
    definition: GetKeysTaskDefinition,
    scope?: vscode.WorkspaceFolder | vscode.TaskScope,
  ) => {
    const args = buildRunArgs(
      { groups: definition.groups ?? [], keys: definition.keys ?? [] },
      definition.command,
      process.platform,
    );
    const task = new vscode.Task(
      definition,
      scope ?? vscode.TaskScope.Workspace,
      definition.command,
      TASK_TYPE,
      new vscode.ProcessExecution(cliPath(), args, { cwd: workspaceCwd() }),
      [],
    );
    task.presentationOptions = { reveal: vscode.TaskRevealKind.Always, clear: true };
    return task;
  };

  const runTask = async () => {
    if (!vscode.workspace.isTrusted) {
      void vscode.window.showWarningMessage(
        'GetKeys: running tasks with keys needs a trusted workspace.',
      );
      return;
    }
    const command = await vscode.window.showInputBox({
      title: 'Run Task with Keys',
      prompt: 'Command to run with keys injected as environment variables',
      placeHolder: 'npm test',
      value: context.workspaceState.get<string>('getkeys.command', ''),
      validateInput: (text) => (text.trim() ? undefined : 'Enter a command'),
    });
    if (!command?.trim()) return;
    await context.workspaceState.update('getkeys.command', command.trim());
    const selection = await pickSelection('Run Task with Keys');
    if (!selection) return;
    const folder = vscode.workspace.workspaceFolders?.[0];
    await vscode.tasks.executeTask(
      createTask({ type: TASK_TYPE, command: command.trim(), ...selection }, folder),
    );
  };

  const configure = () =>
    vscode.commands.executeCommand('workbench.action.openSettings', 'getkeys.cliPath');

  context.subscriptions.push(
    store,
    tree,
    view,
    status,
    vscode.commands.registerCommand('getkeys.reload', () => store.reload()),
    vscode.commands.registerCommand('getkeys.refresh', () => store.refreshRemote()),
    vscode.commands.registerCommand('getkeys.insertRef', insertRef),
    vscode.commands.registerCommand('getkeys.copyKey', copyKey),
    vscode.commands.registerCommand('getkeys.writeEnv', writeEnv),
    vscode.commands.registerCommand('getkeys.runTask', runTask),
    vscode.commands.registerCommand('getkeys.configure', configure),
    vscode.languages.registerHoverProvider([{ scheme: 'file' }, { scheme: 'untitled' }], {
      provideHover(document, position) {
        const line = document.lineAt(position.line).text;
        const found = findRefAt(line, position.character);
        if (!found) return undefined;
        const text = new vscode.MarkdownString(describeRef(store.keys, found.ref), true);
        text.isTrusted = false;
        const range = new vscode.Range(position.line, found.start, position.line, found.end);
        return new vscode.Hover(text, range);
      },
    }),
    vscode.tasks.registerTaskProvider(TASK_TYPE, {
      provideTasks: () => [],
      resolveTask(task) {
        const definition = task.definition as GetKeysTaskDefinition;
        if (typeof definition.command !== 'string' || !definition.command.trim()) return undefined;
        return createTask(definition, task.scope);
      },
    }),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('getkeys.cliPath')) void store.reload();
    }),
    { dispose: () => clearTimeout(clipboardTimer) },
  );

  void store.reload();
}

export function deactivate(): void {
  clearTimeout(clipboardTimer);
}
