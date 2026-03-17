export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | string;
  content: string;
  createdAt?: string;
}
