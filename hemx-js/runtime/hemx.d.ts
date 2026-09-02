export type ResourceKind = "slot" | "handle" | "form";

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

export type InsertPosition =
  | "first"
  | "last"
  | { kind: "before"; target: ResourceRef }
  | { kind: "after"; target: ResourceRef };

export type MovePosition = InsertPosition;
export type PatchMode = "morph" | "replace";
export type ScrollBlock = "start" | "center" | "end" | "nearest";
export type ScrollBehavior = "auto" | "instant" | "smooth";
export type HistoryMode = "push" | "replace";

export type Effect =
  | { kind: "patch"; target: ResourceRef; mode: PatchMode; html: string }
  | { kind: "insert"; target: ResourceRef; position: InsertPosition; html: string }
  | { kind: "remove"; target: ResourceRef }
  | { kind: "move"; target: ResourceRef; position: MovePosition }
  | { kind: "focus"; target: ResourceRef; preventScroll: boolean }
  | { kind: "scroll"; target: ResourceRef; block: ScrollBlock; behavior: ScrollBehavior }
  | { kind: "visit"; url: string; history: HistoryMode }
  | { kind: "dispatch"; event: string; payload: Uint8Array };

export interface EffectBatch {
  abiVersion: number;
  fingerprint: bigint;
  ops: Effect[];
}

export interface HemxRuntime {
  readonly runtimeAbiVersion: number;
  roots(): Element[];
  rootOf(node: Element | null): Element | null;
  applyHtml(html: string, root?: ParentNode | null, title?: string | null): boolean;
  applyBatch(buffer: ArrayBuffer, root?: ParentNode | null): void;
  decodeBatch(buffer: ArrayBuffer): EffectBatch;
}

declare global {
  interface Window {
    hemx: HemxRuntime;
  }
}

declare const hemx: HemxRuntime;
export default hemx;
