import * as React from 'react';

export type MarkTone = 'ink' | 'red' | 'bone' | 'pupil' | 'cobalt' | 'current';
export type ThinkingMotion = 'glance' | 'scan' | 'breathe' | 'sonar';
export type LoadingMotion = 'draw' | 'draw-erase' | 'comet' | 'twin';
export interface LoopMarkProps {
  /** Rendered width in px. Picks the small cut below 32, the large cut from 80. */
  size?: number;
  /** Ink. 'ink' follows the theme. */
  tone?: MarkTone;
  state?: 'idle' | 'thinking' | 'loading';
  /** Variation for the state. Thinking: glance (default), scan, breathe, sonar. Loading: draw-erase (default), draw, comet, twin. */
  motion?: ThinkingMotion | LoadingMotion;
  /** Accessible name; defaults to "Panoptes", "Thinking" or "Loading". */
  title?: string;
  className?: string;
}

export type AppIconId = 'pupil-on-bone' | 'bone-on-pupil' | 'pupil-on-red' | 'red-on-pupil' | 'bone-on-cobalt' | 'cobalt-on-bone' | 'bone-on-red' | 'red-on-bone';
export interface AppIconPickerProps {
  /** Selected icon. Default 'pupil-on-bone'. */
  value?: AppIconId;
  onChange?: (id: AppIconId) => void;
  /** Tile size in px. Default 56. */
  size?: number;
  /** Subset of icons to offer, in picker order. */
  icons?: AppIconId[];
  /** Accessible group name. Default "App icon". */
  label?: string;
  className?: string;
}

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
  size?: 'sm' | 'md' | 'lg';
  /** Lucide icon name from ICON_NAMES. */
  icon?: string;
  /** Shortcut keys shown after the label, e.g. ['⌘', '↵']. */
  kbd?: string[];
  iconOnly?: boolean;
}

export interface KbdProps {
  /** Keys in macOS order, e.g. ['⌘', 'K']. */
  keys: string[];
  /** 'on-primary' inside a primary button; 'inverse' inside a tooltip. */
  tone?: 'on-primary' | 'inverse';
  className?: string;
}

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  hint?: string;
  /** Replaces the hint and marks the field invalid. */
  error?: string;
  /** Leading Lucide icon. */
  icon?: string;
  /** Mono text, for paths, branches and commands. */
  mono?: boolean;
}

export interface TabsProps {
  tabs: { id: string; label: string; icon?: string; count?: number }[];
  value: string;
  onChange?: (id: string) => void;
  className?: string;
}

export type AgentStatus = 'running' | 'waiting' | 'failed' | 'done' | 'idle';
export interface StatusPillProps {
  status: AgentStatus;
  /** Extra detail after the word; never a different status word. */
  children?: React.ReactNode;
  className?: string;
}

export interface AgentRowProps {
  name: string;
  /** One line on what the agent is doing now. */
  task: string;
  status: AgentStatus;
  /** "4m 12s" while running, "12m ago" after. */
  time?: string;
  branch?: string;
  /** You are following this agent live. */
  watching?: boolean;
  selected?: boolean;
  onClick?: () => void;
  className?: string;
}

export interface TerminalProps {
  title?: string;
  /** The agent is typing into this shell now: shows the red Live badge. */
  live?: boolean;
  status?: AgentStatus;
  lines: { kind?: 'prompt' | 'out' | 'dim' | 'ok' | 'err'; text: string; cwd?: string }[];
  className?: string;
}

export interface DiffProps {
  file?: string;
  lines: { type: 'hunk' | 'context' | 'add' | 'remove'; text: string; old?: number; new?: number }[];
  className?: string;
}

export interface IconProps {
  /** One of ICON_NAMES. */
  name: string;
  size?: number;
  strokeWidth?: number;
  /** Accessible name when the icon stands alone. */
  title?: string;
  className?: string;
}

export interface TooltipProps {
  label: string;
  kbd?: string[];
  side?: 'top' | 'bottom';
  /** Force visible; otherwise it shows on hover and focus. */
  open?: boolean;
  children: React.ReactNode;
}

export type MenuItem =
  | { id: string; label: string; icon?: string; kbd?: string[]; danger?: boolean; disabled?: boolean }
  | { separator: true }
  | { heading: string };
export interface MenuProps {
  items: MenuItem[];
  onSelect?: (id: string) => void;
  activeId?: string;
  /** Accessible name. */
  label?: string;
  className?: string;
}

export interface DialogProps {
  title: string;
  children?: React.ReactNode;
  /** Buttons, secondary first, primary last. */
  actions?: React.ReactNode;
  open?: boolean;
  onClose?: () => void;
  /** Width in px. Default 440. */
  width?: number;
  /** Render in place instead of over the window (docs, previews). */
  inline?: boolean;
  className?: string;
}

export interface CommandItem { id: string; label: string; icon?: string; hint?: string; kbd?: string[] }
export interface CommandPaletteProps {
  groups: { label: string; items: CommandItem[] }[];
  onSelect?: (id: string) => void;
  /** Adds an "Ask the agent" row whenever there is a query. */
  onAsk?: (query: string) => void;
  query?: string;
  defaultQuery?: string;
  onQueryChange?: (query: string) => void;
  placeholder?: string;
  onClose?: () => void;
  autoFocus?: boolean;
  className?: string;
}

export interface CheckboxProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label: string;
  hint?: string;
}

export interface SelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  options: (string | { value: string; label: string; disabled?: boolean })[];
  hint?: string;
  error?: string;
  icon?: string;
  mono?: boolean;
}

export declare const Tooltip: React.FC<TooltipProps>;
export declare const Menu: React.FC<MenuProps>;
export declare const Dialog: React.FC<DialogProps>;
export declare const CommandPalette: React.FC<CommandPaletteProps>;
export declare const Checkbox: React.FC<CheckboxProps>;
export declare const Select: React.FC<SelectProps>;
/** The logo's pupil: the sign for running and live. */
export declare const Pupil: React.FC<{ size?: number; className?: string }>;
export declare const LiveBadge: React.FC<{ children?: React.ReactNode; className?: string }>;
export declare const LoopMark: React.FC<LoopMarkProps>;
export declare const AppIconPicker: React.FC<AppIconPickerProps>;
export declare const AppIcon: React.FC<{ id: AppIconId; size?: number; className?: string }>;
export declare const Button: React.FC<ButtonProps>;
export declare const Kbd: React.FC<KbdProps>;
export declare const Input: React.FC<InputProps>;
export declare const Tabs: React.FC<TabsProps>;
export declare const StatusPill: React.FC<StatusPillProps>;
export declare const AgentRow: React.FC<AgentRowProps>;
export declare const Terminal: React.FC<TerminalProps>;
export declare const Diff: React.FC<DiffProps>;
export declare const Icon: React.FC<IconProps>;
export declare const APP_ICONS: { id: AppIconId; label: string; ink: string; ground: string }[];
export declare const ICON_NAMES: string[];
export declare const LOOP_MOTIONS: { thinking: ThinkingMotion[]; loading: LoadingMotion[] };
