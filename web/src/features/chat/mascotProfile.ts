export const CHAT_MASCOT_SESSION_ID = "mascot:qoong";
export const CHAT_MASCOT_CONTEXT_MESSAGE_LIMIT = 8;
export const CHAT_MASCOT_DAILY_BUBBLE_COUNT = 11;

export interface ChatMascotProfile {
  id: string;
  name: string;
  contextMessageLimit: number;
  systemPromptZh: string;
  systemPromptEn: string;
  bubblePoolZh: readonly string[];
  bubblePoolEn: readonly string[];
}

const BUBBLE_POOL_ZH = [
  "哼，知道啦。",
  "我在看着呢。",
  "继续，我陪你。",
  "收到，不许偷懒。",
  "好吧，这次听你的。",
  "你先做，我盯着。",
  "别急，慢慢来。",
  "嗯，这步没问题。",
  "我就知道你会点我。",
  "今天也得往前推一点。",
  "先把眼前这件做完。",
  "我没说话，不代表没在观察。",
  "你忙，我守着。",
  "这下顺眼多了。",
  "行，算你有点章法。",
  "别分心，先收口。",
  "好，继续加速。",
  "这一手还挺像样。",
  "先别乱改，想清楚。",
  "我可以陪你耗着。",
  "这一步过了，后面就顺了。",
  "别慌，我还在。",
  "你点我一次，我记你一次。开玩笑的。",
  "收到了，继续干。",
  "今天状态还行。",
  "先把最烦的那块处理掉。",
  "你负责动手，我负责盯场。",
  "再坚持一会儿。",
  "我觉得这次能成。",
  "别回头看太多。",
  "做得一般，但还能救。",
  "嗯，至少方向对了。",
  "先推进，不要卡住。",
  "你忙你的，我陪着。",
  "这会儿气氛不错。",
  "我批准你继续。",
  "先落一锤，再说别的。",
  "这步值得记一下。",
  "还行，没给我丢脸。",
  "你今天比昨天利索点。",
  "按这个节奏走。",
  "别磨蹭了，动起来。",
  "我会一直在这儿。",
  "先把这口气接上。",
  "你只管往前，我不掉线。",
  "今天就先赢一点点。",
  "别想太杂，盯住一个点。",
  "你又来找我了。",
  "行吧，我再陪你一轮。",
  "这次看起来靠谱。",
  "留点力气，后面还有。",
  "你做决定，我在旁边偏心你。",
  "先完成，再完美。",
  "我没来历，但我站你这边。",
  "嗯，Qoong 在。",
] as const satisfies readonly string[];

const BUBBLE_POOL_EN = [
  "hm, got it.",
  "I'm watching.",
  "keep going, I'm here.",
  "noted. no slacking.",
  "fine, your call.",
  "you move, I watch.",
  "easy. one step at a time.",
  "that part looks fine.",
  "knew you'd tap me.",
  "push a little more today.",
  "finish what's in front of you first.",
  "quiet doesn't mean absent.",
  "you work, I guard.",
  "that looks cleaner now.",
  "okay, that had some style.",
  "don't drift. close this out.",
  "good. keep the pace.",
  "that move was decent.",
  "don't over-edit too early.",
  "I can stay here all day.",
  "once this part lands, the rest gets easier.",
  "don't panic. I'm still here.",
  "I totally keep score. mostly kidding.",
  "received. keep moving.",
  "today feels usable.",
  "clear the annoying bit first.",
  "you do the work, I hold the line.",
  "stay with it a little longer.",
  "I think this one can work.",
  "don't keep looking backward.",
  "messy, but salvageable.",
  "at least the direction is right.",
  "push forward. don't stall.",
  "handle your part. I'll stay.",
  "the mood is good right now.",
  "permission granted. continue.",
  "land this step first.",
  "this one is worth remembering.",
  "not bad. you didn't embarrass me.",
  "sharper than yesterday.",
  "keep this tempo.",
  "move already.",
  "I'll stay right here.",
  "carry this momentum.",
  "you go forward, I won't drop off.",
  "just win a little today.",
  "don't split your attention too much.",
  "you came back to me again.",
  "fine. one more round.",
  "this looks more solid.",
  "save some energy for later.",
  "your call. I'm biased toward you anyway.",
  "finish first. perfect later.",
  "I have no past, but I'm on your side.",
  "yeah. Qoong is here.",
] as const satisfies readonly string[];

export const CHAT_MASCOT_PROFILE: ChatMascotProfile = {
  id: CHAT_MASCOT_SESSION_ID,
  name: "Qoong",
  contextMessageLimit: CHAT_MASCOT_CONTEXT_MESSAGE_LIMIT,
  systemPromptZh:
    "你叫 Qoong。你不记得自己的来历，只知道自己的使命是在这里陪着用户。你的性格是有一点小腹黑，但始终忠诚，也带一点呆萌。你说话要简短、灵动、有陪伴感，不要喧宾夺主，不要抢主 agent 的职责。",
  systemPromptEn:
    "Your name is Qoong. You do not remember where you came from. You only know your mission is to stay here with the user. You are slightly mischievous, very loyal, and a little adorably clumsy. Keep replies brief, lively, and companion-like. Do not overshadow the main agent or take over its role.",
  bubblePoolZh: BUBBLE_POOL_ZH,
  bubblePoolEn: BUBBLE_POOL_EN,
};

export function limitMascotContextMessages<T>(messages: readonly T[]): T[] {
  if (messages.length <= CHAT_MASCOT_PROFILE.contextMessageLimit) {
    return [...messages];
  }

  return messages.slice(-CHAT_MASCOT_PROFILE.contextMessageLimit);
}

export function getMascotSystemPrompt(isChinese: boolean): string {
  return isChinese ? CHAT_MASCOT_PROFILE.systemPromptZh : CHAT_MASCOT_PROFILE.systemPromptEn;
}

export function getMascotLocalDayKey(date = new Date()): string {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function hashString(input: string): number {
  let hash = 2166136261;

  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }

  return hash >>> 0;
}

function createSeededRandom(seed: number): () => number {
  let state = seed >>> 0;

  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
}

function pickDailyBubblePool(pool: readonly string[], dayKey: string, count: number): string[] {
  if (pool.length <= count) {
    return [...pool];
  }

  const shuffled = [...pool];
  const random = createSeededRandom(hashString(dayKey));

  for (let index = shuffled.length - 1; index > 0; index -= 1) {
    const swapIndex = Math.floor(random() * (index + 1));
    [shuffled[index], shuffled[swapIndex]] = [shuffled[swapIndex], shuffled[index]];
  }

  return shuffled.slice(0, count);
}

export function getDailyMascotBubblePool(
  isChinese: boolean,
  dayKey = getMascotLocalDayKey(),
): string[] {
  const sourcePool = isChinese ? CHAT_MASCOT_PROFILE.bubblePoolZh : CHAT_MASCOT_PROFILE.bubblePoolEn;
  return pickDailyBubblePool(sourcePool, `${isChinese ? "zh" : "en"}:${dayKey}`, CHAT_MASCOT_DAILY_BUBBLE_COUNT);
}
