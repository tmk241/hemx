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

/** Remove a live row, or place a row before another (`null`: at the end). */
export type RowStep =
  | { op: "remove"; key: string }
  | { op: "place"; key: string; before: string | null };

export type PatchMode = "morph" | "replace";
export type ScrollBlock = "start" | "center" | "end" | "nearest";
export type ScrollBehavior = "auto" | "instant" | "smooth";
export type HistoryMode = "push" | "replace";

export type Effect =
  | { kind: "patch"; target: ResourceRef; mode: PatchMode; html: string }
  /** The collection holds exactly `order`; `rows` supplies new or changed rows. */
  | { kind: "rows"; target: ResourceRef; order: string[]; rows: [key: string, html: string][] }
  /** Change the collection from the order hash `from` to `to` by applying `steps`. */
  | { kind: "rowsDelta"; target: ResourceRef; from: bigint; to: bigint; steps: RowStep[]; rows: [key: string, html: string][] }
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
  applyBatch(buffer: ArrayBuffer, root?: ParentNode | null): boolean;
  decodeBatch(buffer: ArrayBuffer): EffectBatch;
}

declare global {
  interface Window {
    hemx: HemxRuntime;
  }
}

declare const hemx: HemxRuntime;
export default hemx;
