import {
  Blocks,
  Building2,
  Gamepad2,
  Hash,
  Mail,
  MessageCircleMore,
  MessageSquare,
  PanelsTopLeft,
  Send,
  Smartphone,
  Webhook,
} from "lucide-react";

interface ChannelSurfaceIconProps {
  id: string;
  label: string;
}

function pickChannelIcon(id: string, label: string) {
  const normalized = `${id} ${label}`.toLowerCase();

  if (normalized.includes("telegram")) return Send;
  if (normalized.includes("feishu") || normalized.includes("lark")) return MessageSquare;
  if (normalized.includes("matrix")) return Hash;
  if (normalized.includes("wecom") || normalized.includes("wechat")) return Building2;
  if (normalized.includes("discord")) return Gamepad2;
  if (normalized.includes("slack")) return PanelsTopLeft;
  if (normalized.includes("whatsapp")) return MessageCircleMore;
  if (normalized.includes("email")) return Mail;
  if (normalized.includes("webhook")) return Webhook;
  if (normalized.includes("teams")) return PanelsTopLeft;
  if (normalized.includes("imessage")) return Smartphone;
  if (normalized.includes("dingtalk") || normalized.includes("google_chat")) return MessageSquare;

  return Blocks;
}

export function ChannelSurfaceIcon({ id, label }: ChannelSurfaceIconProps) {
  const Icon = pickChannelIcon(id, label);

  return (
    <span className="abilities-channel-icon" aria-hidden="true">
      <Icon size={17} strokeWidth={1.9} />
    </span>
  );
}
