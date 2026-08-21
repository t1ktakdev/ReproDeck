import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;
const base = (props: IconProps) => ({ width: 16, height: 16, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.7, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, ...props });

export const HomeIcon = (props: IconProps) => <svg {...base(props)}><path d="M3.5 10.5 12 3l8.5 7.5"/><path d="M5.5 9.5V21h13V9.5"/><path d="M9.5 21v-6h5v6"/></svg>;
export const SessionIcon = (props: IconProps) => <svg {...base(props)}><rect x="4" y="4" width="16" height="16" rx="2"/><path d="M8 9h8M8 13h8M8 17h5"/></svg>;
export const RepoIcon = (props: IconProps) => <svg {...base(props)}><circle cx="6" cy="5" r="2"/><circle cx="18" cy="6" r="2"/><circle cx="8" cy="19" r="2"/><path d="M6 7v5a7 7 0 0 0 7 7h-3M8 5h8M18 8v3a8 8 0 0 1-8 8"/></svg>;
export const CapsuleIcon = (props: IconProps) => <svg {...base(props)}><path d="M9 3h6l4 4v10l-4 4H9l-4-4V7z"/><path d="M9 8h6M9 12h6M9 16h4"/></svg>;
export const SettingsIcon = (props: IconProps) => <svg {...base(props)}><circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.5 1A7 7 0 0 0 14.3 5L14 2h-4l-.3 3a7 7 0 0 0-2.1 1.9l-2.5-1-2 3.4 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.4 2.5-1A7 7 0 0 0 9.7 19l.3 3h4l.3-3a7 7 0 0 0 2.1-1.9l2.5 1 2-3.4-2-1.5c.1-.4.1-.8.1-1.2Z"/></svg>;
export const TimelineIcon = (props: IconProps) => <svg {...base(props)}><path d="M12 8v5l3 2"/><circle cx="12" cy="12" r="9"/></svg>;
export const ChangesIcon = (props: IconProps) => <svg {...base(props)}><path d="M7 3v14M7 17l-3-3M7 17l3-3M17 21V7M17 7l-3 3M17 7l3 3"/></svg>;
export const EvidenceIcon = (props: IconProps) => <svg {...base(props)}><path d="M6 3h9l3 3v15H6z"/><path d="M15 3v4h4M9 11h6M9 15h6"/></svg>;
export const EnvIcon = (props: IconProps) => <svg {...base(props)}><path d="M8 3h8v4l3 5v7H5v-7l3-5z"/><path d="M8 7h8M9 13h6"/></svg>;
export const VerifyIcon = (props: IconProps) => <svg {...base(props)}><path d="m5 12 4 4L19 6"/></svg>;
export const OverviewIcon = (props: IconProps) => <svg {...base(props)}><rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/></svg>;
export const PlayIcon = (props: IconProps) => <svg {...base(props)}><path d="m8 5 11 7-11 7z"/></svg>;
export const PlusIcon = (props: IconProps) => <svg {...base(props)}><path d="M12 5v14M5 12h14"/></svg>;
export const SearchIcon = (props: IconProps) => <svg {...base(props)}><circle cx="11" cy="11" r="6"/><path d="m16 16 4 4"/></svg>;
export const ExternalIcon = (props: IconProps) => <svg {...base(props)}><path d="M14 5h5v5M19 5l-8 8"/><path d="M18 13v6H5V6h6"/></svg>;
export const CheckIcon = (props: IconProps) => <svg {...base(props)}><path d="m5 12 4 4L19 6"/></svg>;
export const WarningIcon = (props: IconProps) => <svg {...base(props)}><path d="M12 3 2.8 20h18.4z"/><path d="M12 9v4M12 17h.01"/></svg>;
export const PanelIcon = (props: IconProps) => <svg {...base(props)}><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/></svg>;
export const InspectorIcon = (props: IconProps) => <svg {...base(props)}><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/></svg>;
export const CloseIcon = (props: IconProps) => <svg {...base(props)}><path d="m7 7 10 10M17 7 7 17"/></svg>;
export const CommandIcon = (props: IconProps) => <svg {...base(props)}><path d="M6 8h12M6 12h8M6 16h10"/><path d="m17 14 2 2-2 2"/></svg>;
