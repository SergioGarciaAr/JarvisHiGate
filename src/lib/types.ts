// Mirrors of the serde types in crates/jarvis-core and src-tauri.

export type DiagnosticoScope = "general" | "full" | "red" | "ram";

export type Subcommand = "piensa" | "diagnostico" | "conversa" | "claw" | "ayuda";

export type Command =
  | { command: "piensa"; mode: "plan" | "comparar"; prompt: string }
  | { command: "diagnostico"; scope: DiagnosticoScope }
  | { command: "conversa"; mode: "voz" | "manos_libres" }
  | { command: "claw"; task: string }
  | { command: "ayuda"; topic: Subcommand | null };

export interface ParseErrorInfo {
  kind: string;
  message: string;
  hint: string;
}

export interface OllamaErrorInfo {
  kind: string;
  message: string;
  recovery: string;
  retryable: boolean;
}

export interface ModelInfo {
  name: string;
  size: number;
  capabilities: string[];
}

export type OllamaHealth =
  | { status: "available"; version: string; models: ModelInfo[] }
  | { status: "no_models"; version: string; recovery: string }
  | { status: "unavailable"; error: OllamaErrorInfo };

export type ThinkMode = "plan" | "comparar" | "analisis";

export type SubmitOutcome =
  | { kind: "empty" }
  | { kind: "help"; text: string }
  | { kind: "parse_error"; error: ParseErrorInfo }
  | { kind: "chat_started"; request_id: string; model: string }
  | { kind: "run_started"; request_id: string; goal: string; mode: ThinkMode; model: string }
  | { kind: "diagnostic_started"; request_id: string; scope: DiagnosticoScope }
  | { kind: "not_available"; command: Command; phase: number; reason: string; alternative: string }
  | { kind: "no_model"; message: string; recovery: string };

export type ChatEvent =
  | { kind: "delta"; request_id: string; text: string }
  | { kind: "done"; request_id: string }
  | { kind: "cancelled"; request_id: string }
  | { kind: "error"; request_id: string; error: OllamaErrorInfo };

export type EventKind =
  | "goal"
  | "phase"
  | "plan"
  | "tool_call"
  | "tool_result"
  | "result"
  | "error"
  | "retry"
  | "next_step"
  | "completed"
  | "cancelled";

export type EventStatus = "pending" | "running" | "ok" | "failed" | "cancelled";

export interface ProgressEvent {
  id: string;
  run_id: string;
  seq: number;
  kind: EventKind;
  status: EventStatus;
  timestamp: string;
  summary: string;
  data?: unknown;
}

export interface Paso {
  titulo: string;
  detalle: string;
}

export type ThinkResult =
  | { mode: "plan"; objetivo: string; resumen: string; pasos: Paso[]; riesgos: string[]; siguiente_paso: string }
  | {
      mode: "comparar";
      objetivo: string;
      opciones: { nombre: string; ventajas: string[]; inconvenientes: string[] }[];
      recomendacion: string;
      justificacion: string;
    }
  | { mode: "analisis"; objetivo: string; resumen: string; hallazgos: string[]; siguiente_paso: string };

export interface Unavailable {
  metric: string;
  reason: string;
}

export interface TelemetrySnapshot {
  timestamp: string;
  hostname: string | null;
  os_version: string | null;
  uptime_secs: number;
  cpu_percent: number | null;
  cpu_cores: number;
  load_avg: [number, number, number] | null;
  mem_total: number | null;
  mem_used: number | null;
  mem_available: number | null;
  swap_total: number | null;
  swap_used: number | null;
  disk: { mount: string; total_bytes: number; available_bytes: number } | null;
  net_rx_per_sec: number | null;
  net_tx_per_sec: number | null;
  unavailable: Unavailable[];
}

export interface Settings {
  shortcut: string;
  model: string | null;
  voice: string | null;
  autospeak: boolean;
  tts_engine: TtsEngineId | null;
  neural_voice: string | null;
}

export interface ShortcutStatus {
  shortcut: string;
  registered: boolean;
  error: string | null;
}

// ---- Diagnostics (crates/jarvis-core/src/diagnostics) ----

export type FindingKind = "problem" | "warning" | "observation";
export type Severity = "info" | "low" | "medium" | "high" | "critical";
export type Confidence = "high" | "medium" | "low";

export interface Evidence {
  label: string;
  value: string;
  source: string;
}

export interface Finding {
  id: string;
  kind: FindingKind;
  severity: Severity;
  title: string;
  evidence: Evidence[];
  confidence: Confidence;
  recommendation: string;
  proposed_action: string | null;
}

export interface DataGap {
  what: string;
  source: string;
  reason: string;
}

export interface MemoryData {
  total: number | null;
  used: number | null;
  available: number | null;
  pressure: "normal" | "warn" | "critical" | null;
  free_level: number | null;
  vm: { page_size: number; compressor_occupied: number; compressed_stored: number; wired: number; swapouts: number } | null;
  swap: { total: number; used: number; free: number; encrypted: boolean } | null;
  processes: { top: { pid: number; name: string; memory: number }[]; total: number; without_data: number } | null;
}

export interface InterfaceInfo {
  name: string;
  mac: string;
  mtu: number;
  state: string;
  addresses: string[];
  has_routable_ip: boolean;
  rx_errors: number;
  tx_errors: number;
}

export type TcpOutcome = "connected" | "refused" | "timeout" | "unreachable" | "error";

export interface NetworkData {
  interfaces: InterfaceInfo[];
  services: [string, string][];
  route: { status: "unknown" } | { status: "no_default" } | { status: "default"; gateway: string; interface: string | null };
  dns_servers: string[] | null;
  gateway: { gateway: string; stats: { transmitted: number; received: number; avg_ms: number | null; send_error: string | null } } | null;
  dns: { host: string; addresses: string[]; ms: number | null; error: string | null }[];
  tcp: { label: string; target: string; via_dns: boolean; outcome: TcpOutcome; ms: number | null; detail: string | null }[];
}

export interface DiagnosticReport {
  scope: DiagnosticoScope;
  generated_at: string;
  duration_ms: number;
  system: TelemetrySnapshot | null;
  memory: MemoryData | null;
  network: NetworkData | null;
  findings: Finding[];
  gaps: DataGap[];
  notes: string[];
}

// ---- Voice (crates/jarvis-core/src/voice, src-tauri) ----

export interface Voice {
  name: string;
  locale: string;
  sample: string;
}

export interface SttEngineStatus {
  engine: "whisper_cpp" | "apple_speech_transcriber";
  status: "not_installed" | "model_missing" | "not_integrated";
  detail: string;
}

export interface VoiceStatus {
  tts: { available: boolean; voices: Voice[]; selected: string | null; reason: string | null };
  stt: { available: boolean; reason: string; engines: SttEngineStatus[] };
  autospeak: boolean;
  /** Engine in use for speech synthesis. */
  engine: TtsEngineId;
  engines: TtsEngineInfo[];
  /** Reference voices for the neural engine. */
  neural_voices: string[];
  neural_voice: string;
}

export type TtsEngineId = "neural" | "say";

export interface TtsEngineInfo {
  id: TtsEngineId;
  label: string;
  available: boolean;
  reason: string | null;
}

export type VoiceEvent =
  | { kind: "started"; utterance: number; request_id: string }
  | { kind: "finished"; utterance: number; request_id: string }
  | { kind: "stopped"; utterance: number; request_id: string }
  | { kind: "error"; utterance: number; request_id: string; message: string };
