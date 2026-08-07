export type ResourceKind = "slot" | "atom" | "handle" | "form";

export interface ResourceId {
  kind: ResourceKind;
  id: number;
}

export type ScopeKey =
  | { kind: "key"; value: string }
  | { kind: "field"; value: string };

export interface ResourceRef {
  resource: ResourceId;
  scope: ScopeKey | null;
}

export type Payload =
  | { kind: "text"; value: string }
  | { kind: "html"; value: string };

export type ScrollBehavior =
  | "preserve"
  | "top"
  | { kind: "element"; target: ResourceRef };

export type Effect =
  | { kind: "put"; target: ResourceRef; payload: Payload }
  | { kind: "insert"; target: ResourceRef; key: string; payload: Payload }
  | { kind: "prepend"; target: ResourceRef; key: string; payload: Payload }
  | { kind: "remove"; target: ResourceRef; key: string | null }
  | { kind: "move"; target: ResourceRef; key: string; before: string | null }
  | { kind: "focus"; target: ResourceRef }
  | { kind: "navigate"; url: string; mode: "push" | "replace" | "redirect"; scroll: ScrollBehavior; title: string | null }
  | { kind: "emit"; name: string; payload: string };

export interface EffectBatch {
  abiVersion: number;
  fingerprint: bigint;
  ops: Effect[];
}

export interface AtomSnapshot {
  id: number;
  bytes: Uint8Array;
}

export type ClientHandler = (
  eventVersion: number,
  eventKind: string,
  eventValue: string | undefined,
  eventChecked: boolean | undefined,
  eventKey: string | undefined,
  stateVersion: number,
  encodedState: string,
) => Uint8Array | Promise<Uint8Array>;

export interface HemxRuntime {
  readonly runtimeAbiVersion: number;
  roots(): Element[];
  rootOf(node: Element | null): Element | null;
  applyHtml(html: string, root?: ParentNode | null, title?: string | null): boolean;
  applyBatch(buffer: ArrayBuffer, root?: ParentNode | null): void;
  decodeBatch(buffer: ArrayBuffer): EffectBatch;
  atomValue(root: Element | ParentNode | null | undefined, id: number): Uint8Array | undefined;
  decodeAtomState(encoded: string): AtomSnapshot[];
  registerClientHandler(name: string, handler: ClientHandler): ClientHandler | undefined;
}

declare global {
  interface Window {
    hemx: HemxRuntime;
  }
}

declare const hemx: HemxRuntime;
export default hemx;
