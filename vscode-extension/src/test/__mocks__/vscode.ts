/**
 * Mock VS Code API for unit testing.
 * Provides minimal implementations of commonly used VS Code types.
 */

export enum TreeItemCollapsibleState {
  None = 0,
  Collapsed = 1,
  Expanded = 2,
}

export enum StatusBarAlignment {
  Left = 1,
  Right = 2,
}

export class TreeItem {
  label: string;
  collapsibleState: TreeItemCollapsibleState;
  iconPath?: ThemeIcon;
  tooltip?: string | MarkdownString;
  description?: string;
  contextValue?: string;
  command?: Command;

  constructor(
    label: string,
    collapsibleState: TreeItemCollapsibleState = TreeItemCollapsibleState.None
  ) {
    this.label = label;
    this.collapsibleState = collapsibleState;
  }
}

export class ThemeIcon {
  id: string;
  color?: ThemeColor;

  constructor(id: string, color?: ThemeColor) {
    this.id = id;
    this.color = color;
  }
}

export class ThemeColor {
  id: string;

  constructor(id: string) {
    this.id = id;
  }
}

export class MarkdownString {
  value: string;
  isTrusted: boolean;

  constructor(value?: string) {
    this.value = value ?? "";
    this.isTrusted = false;
  }

  appendMarkdown(value: string): MarkdownString {
    this.value += value;
    return this;
  }

  appendText(value: string): MarkdownString {
    this.value += value;
    return this;
  }
}

export class EventEmitter<T> {
  private listeners: Array<(e: T) => void> = [];

  event = (listener: (e: T) => void): Disposable => {
    this.listeners.push(listener);
    return {
      dispose: () => {
        const index = this.listeners.indexOf(listener);
        if (index >= 0) {
          this.listeners.splice(index, 1);
        }
      },
    };
  };

  fire(data: T): void {
    // Iterate over copy to handle listener self-disposal during iteration
    for (const listener of [...this.listeners]) {
      listener(data);
    }
  }

  dispose(): void {
    this.listeners = [];
  }
}

export class Uri {
  static file(path: string): Uri {
    return new Uri("file", "", path, "", "");
  }

  static parse(value: string): Uri {
    // Handle custom schemes like file://, vscode://, git://
    const schemeMatch = value.match(/^([a-zA-Z][a-zA-Z0-9+.-]*):\/\//);
    if (schemeMatch) {
      const scheme = schemeMatch[1];
      const rest = value.slice(schemeMatch[0].length);

      // Split off fragment
      const [withoutFragment, fragment = ""] = rest.split("#");
      // Split off query
      const [pathWithAuthority, query = ""] = withoutFragment.split("?");
      // Split authority from path
      const slashIndex = pathWithAuthority.indexOf("/");
      const authority = slashIndex >= 0 ? pathWithAuthority.slice(0, slashIndex) : pathWithAuthority;
      const path = slashIndex >= 0 ? pathWithAuthority.slice(slashIndex) : "";

      return new Uri(scheme, authority, path, query, fragment);
    }

    // Fallback for relative paths or unknown formats
    return new Uri("file", "", value, "", "");
  }

  static joinPath(base: Uri, ...pathSegments: string[]): Uri {
    const joined = [base.path, ...pathSegments].join("/").replace(/\/+/g, "/");
    return new Uri(base.scheme, base.authority, joined, base.query, base.fragment);
  }

  constructor(
    public scheme: string,
    public authority: string,
    public path: string,
    public query: string,
    public fragment: string
  ) {}

  get fsPath(): string {
    return this.path;
  }

  toString(): string {
    let result = `${this.scheme}://`;
    if (this.authority) {
      result += this.authority;
    }
    result += this.path;
    if (this.query) {
      result += `?${this.query}`;
    }
    if (this.fragment) {
      result += `#${this.fragment}`;
    }
    return result;
  }
}

export interface Command {
  command: string;
  title: string;
  arguments?: unknown[];
}

export interface Disposable {
  dispose(): void;
}

export interface OutputChannel {
  appendLine(value: string): void;
  append(value: string): void;
  clear(): void;
  show(): void;
  hide(): void;
  dispose(): void;
}

export interface StatusBarItem {
  text: string;
  tooltip?: string | MarkdownString;
  command?: string | Command;
  name?: string;
  show(): void;
  hide(): void;
  dispose(): void;
}

// Mock workspace
export const workspace = {
  workspaceFolders: [
    {
      uri: Uri.file("/test/workspace"),
      name: "test",
      index: 0,
    },
  ],
  getConfiguration: () => ({
    get: <T>(key: string, defaultValue: T): T => defaultValue,
  }),
  onDidChangeConfiguration: (): Disposable => ({ dispose: () => {} }),
  createFileSystemWatcher: () => ({
    onDidChange: (): Disposable => ({ dispose: () => {} }),
    onDidCreate: (): Disposable => ({ dispose: () => {} }),
    onDidDelete: (): Disposable => ({ dispose: () => {} }),
    dispose: () => {},
  }),
};

// Mock window
export const window = {
  createOutputChannel: (): OutputChannel => ({
    appendLine: () => {},
    append: () => {},
    clear: () => {},
    show: () => {},
    hide: () => {},
    dispose: () => {},
  }),
  createTreeView: () => ({
    dispose: () => {},
  }),
  createStatusBarItem: (): StatusBarItem => ({
    text: "",
    show: () => {},
    hide: () => {},
    dispose: () => {},
  }),
  showInformationMessage: async () => undefined,
  showWarningMessage: async () => undefined,
  showErrorMessage: async () => undefined,
  showInputBox: async () => undefined,
  showQuickPick: async () => undefined,
  withProgress: async <T>(
    _options: unknown,
    task: (progress: unknown, token: unknown) => Thenable<T>
  ) => task(
    { report: () => {} },
    { isCancellationRequested: false, onCancellationRequested: () => ({ dispose: () => {} }) }
  ),
  createTerminal: () => ({
    sendText: () => {},
    show: () => {},
    dispose: () => {},
  }),
};

// Mock commands
export const commands = {
  registerCommand: (): Disposable => ({ dispose: () => {} }),
  executeCommand: async () => undefined,
};

// Mock extensions
export const extensions = {
  getExtension: () => undefined,
};

// Mock env
export const env = {
  openExternal: async () => true,
};

// Mock progress location
export enum ProgressLocation {
  SourceControl = 1,
  Window = 10,
  Notification = 15,
}

// Mock relative pattern
export class RelativePattern {
  constructor(
    public base: string | { uri: Uri },
    public pattern: string
  ) {}
}
