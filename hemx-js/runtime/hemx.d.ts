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

/** Stable discriminants on the existing root-scoped hemx:error event. */
export type HemxErrorCode =
  | "HEMX_REQUEST_FAILED"
  | "HEMX_STALE_BUILD"
  | "HEMX_BATCH_FAILED"
  | "HEMX_ROWS_DRIFT"
  | "HEMX_UNSUPPORTED_ABI"
  | "HEMX_RUNTIME_FAILED";

export interface HemxErrorDetail {
  code: HemxErrorCode;
  /** Diagnostic text, not a translation key or a write-outcome guarantee. */
  message: string;
  /** HTTP status when available; null for failures without one. */
  status: number | null;
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
  interface ElementEventMap {
    "hemx:error": CustomEvent<HemxErrorDetail>;
  }
  interface DocumentEventMap {
    "hemx:error": CustomEvent<HemxErrorDetail>;
  }
  interface WindowEventMap {
    "hemx:error": CustomEvent<HemxErrorDetail>;
  }
  interface Window {
    hemx: HemxRuntime;
  }
}

declare const hemx: HemxRuntime;
export default hemx;
