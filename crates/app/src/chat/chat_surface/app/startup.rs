fn startup_onboarding_enabled(runtime: &CliTurnRuntime) -> bool {
    startup_env_truthy("LOONG_TUI_ONBOARD")
        || (runtime.config.provider.api_key().is_none()
            && runtime.config.provider.oauth_access_token().is_none())
}

fn startup_env_truthy(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn build_startup_provider_options(
    runtime: &CliTurnRuntime,
    language: Language,
) -> Vec<StartupProviderOption> {
    let current_provider_kind = runtime.config.provider.kind;
    ProviderKind::all_sorted()
        .iter()
        .map(|kind| {
            let is_current = *kind == current_provider_kind;
            let provider = ProviderConfig {
                kind: *kind,
                ..ProviderConfig::default()
            };
            let auth_env_name = provider
                .auth_hint_env_names()
                .into_iter()
                .find(|env_name| std::env::var_os(env_name).is_some());
            let detail = if is_current {
                startup_current_provider_detail(runtime, language)
            } else if let Some(env_name) = auth_env_name.as_deref() {
                startup_provider_migration_detail(kind.display_name(), env_name, language)
            } else {
                startup_provider_kind_detail(kind.display_name(), language)
            };

            StartupProviderOption {
                kind: *kind,
                auth_env_name,
                is_current,
                label: kind.display_name().to_owned(),
                detail,
                recommended: is_current,
            }
        })
        .collect()
}

fn startup_current_provider_detail(runtime: &CliTurnRuntime, language: Language) -> String {
    if let Some(env_name) = runtime.config.provider.resolved_auth_env_name() {
        return match language {
            Language::ZhCn => {
                format!("沿用 config.toml 里的当前 Loong provider。凭证目前通过 {env_name} 解析。")
            }
            Language::ZhTw => {
                format!("沿用 config.toml 裡目前的 Loong provider。憑證目前透過 {env_name} 解析。")
            }
            Language::Ja => format!(
                "config.toml の現在の Loong provider をそのまま使います。認証情報は今 {env_name} から解決されています。"
            ),
            Language::Ru => format!(
                "использовать текущий provider из config.toml. Сейчас учётные данные берутся через {env_name}."
            ),
            Language::En => format!(
                "reuse the active Loong provider from config.toml. credentials currently resolve through {env_name}."
            ),
        };
    }

    if runtime.config.provider.api_key().is_some()
        || runtime.config.provider.oauth_access_token().is_some()
    {
        return match language {
            Language::ZhCn => {
                "沿用 config.toml 里的当前 Loong provider。当前 runtime 已经拿到了 provider 凭证。"
                    .to_owned()
            }
            Language::ZhTw => {
                "沿用 config.toml 裡目前的 Loong provider。當前 runtime 已經拿到 provider 憑證。"
                    .to_owned()
            }
            Language::Ja => {
                "config.toml の現在の Loong provider をそのまま使います。いまの runtime には認証情報が読み込まれています。"
                    .to_owned()
            }
            Language::Ru => {
                "использовать текущий provider из config.toml. В текущем runtime учётные данные уже загружены."
                    .to_owned()
            }
            Language::En => {
                "reuse the active Loong provider from config.toml. the current runtime already has provider credentials loaded."
                    .to_owned()
            }
        };
    }

    match language {
        Language::ZhCn => {
            "沿用 config.toml 里的当前 provider 形状。第一轮真实对话前仍需要把凭证接好。"
                .to_owned()
        }
        Language::ZhTw => {
            "沿用 config.toml 裡目前的 provider 形狀。第一輪真實對話前仍需要把憑證接好。"
                .to_owned()
        }
        Language::Ja => {
            "config.toml の現在の provider 形を引き継ぎます。最初の実際のターンの前に認証情報の配線はまだ必要です。"
                .to_owned()
        }
        Language::Ru => {
            "сохранить текущую форму provider из config.toml. Перед первым реальным ходом авторизацию ещё нужно подключить."
                .to_owned()
        }
        Language::En => {
            "reuse the current provider shape from config.toml. credentials still need to be wired before the first real turn."
                .to_owned()
        }
    }
}

fn startup_provider_migration_detail(
    provider_label: &str,
    env_name: &str,
    language: Language,
) -> String {
    match language {
        Language::ZhCn => format!(
            "Loong 在 {env_name} 里发现了可复用的 {provider_label} 凭证。这里先接上 provider，剩余细节后面再回到 config.toml。"
        ),
        Language::ZhTw => format!(
            "Loong 在 {env_name} 裡發現了可重用的 {provider_label} 憑證。這裡先接上 provider，剩餘細節之後再回到 config.toml。"
        ),
        Language::Ja => format!(
            "Loong は {env_name} に再利用できる {provider_label} 認証を見つけました。ここでは provider だけ先につなぎ、残りの細部は後で config.toml に戻って詰められます。"
        ),
        Language::Ru => format!(
            "Loong нашёл готовые учётные данные {provider_label} в {env_name}. Здесь можно сначала выбрать provider, а остальное позже довести в config.toml."
        ),
        Language::En => format!(
            "Loong found a ready local {provider_label} credential in {env_name}. You can keep moving here and wire the rest later in config.toml."
        ),
    }
}

fn startup_provider_kind_detail(provider_label: &str, language: Language) -> String {
    match language {
        Language::ZhCn => format!(
            "先切到 {provider_label}；后续 base_url、model 和鉴权还可以回到 config.toml 里细调。"
        ),
        Language::ZhTw => format!(
            "先切到 {provider_label}；後續 base_url、model 和驗證還可以回到 config.toml 裡微調。"
        ),
        Language::Ja => format!(
            "先に {provider_label} へ切り替え、base_url・model・認証の細部はあとで config.toml に戻って詰められます。"
        ),
        Language::Ru => format!(
            "Сначала переключитесь на {provider_label}; base_url, model и авторизацию потом можно дочистить в config.toml."
        ),
        Language::En => format!(
            "switch to {provider_label} now; you can still fine-tune base_url, model, and auth later in config.toml."
        ),
    }
}

fn persist_startup_provider_selection(
    runtime: &mut CliTurnRuntime,
    option: StartupProviderOption,
    language: Language,
) -> CliResult<String> {
    let mut config = runtime.config.clone();
    let path = runtime.resolved_path.display().to_string();
    let mut provider = if option.is_current {
        config.provider.clone()
    } else {
        ProviderConfig {
            kind: option.kind,
            ..ProviderConfig::default()
        }
        .selection_baseline()
    };

    if !option.is_current
        && let Some(env_name) = option.auth_env_name.as_deref()
    {
        match option.kind.auth_scheme() {
            ProviderAuthScheme::Bearer => {
                provider.set_oauth_access_token_env_binding(Some(env_name.to_owned()));
            }
            ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => {
                provider.set_api_key_env_binding(Some(env_name.to_owned()));
            }
        }
    }
    let profile = ProviderProfileConfig::from_provider(provider.clone());
    if config.active_provider_id().map(str::to_owned) != Some(provider.inferred_profile_id()) {
        config.last_provider = config.active_provider_id().map(str::to_owned);
    }
    config.set_active_provider_profile(provider.inferred_profile_id(), profile);

    crate::config::write(Some(path.as_str()), &config, true)?;
    runtime.config = config;

    let summary = if let Some(env_name) = option.auth_env_name.as_deref() {
        match language {
            Language::ZhCn => format!("provider 已保存：{}（复用 {env_name}）。", option.label),
            Language::ZhTw => format!("provider 已保存：{}（重用 {env_name}）。", option.label),
            Language::Ja => format!(
                "provider を保存しました: {}（{env_name} を再利用）。",
                option.label
            ),
            Language::Ru => format!(
                "provider сохранён: {} (переиспользуется {env_name}).",
                option.label
            ),
            Language::En => format!("provider saved: {} (reusing {env_name}).", option.label),
        }
    } else {
        match language {
            Language::ZhCn => format!("provider 已保存：{}。", option.label),
            Language::ZhTw => format!("provider 已保存：{}。", option.label),
            Language::Ja => format!("provider を保存しました: {}。", option.label),
            Language::Ru => format!("provider сохранён: {}.", option.label),
            Language::En => format!("provider saved: {}.", option.label),
        }
    };

    Ok(summary)
}

fn startup_language_label(language: Language) -> &'static str {
    match language {
        Language::En => "English",
        Language::ZhCn => "简体中文",
        Language::ZhTw => "繁體中文",
        Language::Ja => "日本語",
        Language::Ru => "Русский",
    }
}

fn startup_onboarding_footer_text(stage: StartupOnboardingStage) -> &'static str {
    match stage {
        StartupOnboardingStage::Skills => "↑/↓ move · Space toggle · Enter continue · Esc back",
        StartupOnboardingStage::Language
        | StartupOnboardingStage::Provider
        | StartupOnboardingStage::SetupPath
        | StartupOnboardingStage::Personalization => "↑/↓ move · Enter continue · Esc back",
        StartupOnboardingStage::Finish => "Enter start chatting · Esc back",
    }
}

fn startup_onboarding_footer_text_for_language(
    stage: StartupOnboardingStage,
    language: Language,
) -> &'static str {
    match language {
        Language::ZhCn => match stage {
            StartupOnboardingStage::Skills => "↑/↓ 移动 · Space 勾选 · Enter 继续 · Esc 返回",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ 移动 · Enter 继续 · Esc 返回",
            StartupOnboardingStage::Finish => "Enter 开始聊天 · Esc 返回",
        },
        Language::ZhTw => match stage {
            StartupOnboardingStage::Skills => "↑/↓ 移動 · Space 勾選 · Enter 繼續 · Esc 返回",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ 移動 · Enter 繼續 · Esc 返回",
            StartupOnboardingStage::Finish => "Enter 開始聊天 · Esc 返回",
        },
        Language::Ja => match stage {
            StartupOnboardingStage::Skills => "↑/↓ move · Space 選択 · Enter 続行 · Esc 戻る",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ move · Enter 続行 · Esc 戻る",
            StartupOnboardingStage::Finish => "Enter で会話開始 · Esc 戻る",
        },
        Language::Ru => match stage {
            StartupOnboardingStage::Skills => "↑/↓ move · Space выбор · Enter дальше · Esc назад",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ move · Enter дальше · Esc назад",
            StartupOnboardingStage::Finish => "Enter начать чат · Esc назад",
        },
        _ => startup_onboarding_footer_text(stage),
    }
}

fn startup_onboarding_subtitle(stage: StartupOnboardingStage, language: Language) -> &'static str {
    match language {
        Language::ZhCn => match stage {
            StartupOnboardingStage::Language => "先选 TUI 语言，之后仍可继续细调 config.toml。",
            StartupOnboardingStage::Provider => {
                "先选 Loong 优先准备的 provider，本地可复用的凭证会自动显示出来。"
            }
            StartupOnboardingStage::Skills => {
                "Loong 可以预装少量 bundled skills。Space 勾选，Enter 继续。"
            }
            StartupOnboardingStage::SetupPath => {
                "决定是保持 shell 极简，还是继续看 provider、web search、channels、MCP 这些后续路径。"
            }
            StartupOnboardingStage::Personalization => {
                "先存一个首轮对话风格，让第一条真正回复更贴近你想要的节奏。"
            }
            StartupOnboardingStage::Finish => {
                "现在可以直接开聊；MCP、web provider、channel、personalization 也可以按需要再继续。"
            }
        },
        Language::ZhTw => match stage {
            StartupOnboardingStage::Language => "先選 TUI 語言，之後仍可繼續微調 config.toml。",
            StartupOnboardingStage::Provider => {
                "先選 Loong 優先準備的 provider，本地可重用的憑證會自動顯示出來。"
            }
            StartupOnboardingStage::Skills => {
                "Loong 可以預裝少量 bundled skills。Space 勾選，Enter 繼續。"
            }
            StartupOnboardingStage::SetupPath => {
                "決定是保持 shell 極簡，還是繼續看 provider、web search、channels、MCP 這些後續路徑。"
            }
            StartupOnboardingStage::Personalization => {
                "先存一個首輪對話風格，讓第一條真正回覆更貼近你想要的節奏。"
            }
            StartupOnboardingStage::Finish => {
                "現在可以直接開聊；MCP、web provider、channel、personalization 也可以按需要再繼續。"
            }
        },
        Language::Ja => match stage {
            StartupOnboardingStage::Language => {
                "先に TUI の言語を選びます。あとで config.toml は引き続き細かく調整できます。"
            }
            StartupOnboardingStage::Provider => {
                "Loong が先に整える provider を選びます。再利用できるローカル認証は自動で見えます。"
            }
            StartupOnboardingStage::Skills => {
                "Loong は少数の bundled skills を先に入れられます。Space で選択し、Enter で進みます。"
            }
            StartupOnboardingStage::SetupPath => {
                "shell を最小限に保つか、provider、web search、channels、MCP の続き方まで今のうちに見るかを決めます。"
            }
            StartupOnboardingStage::Personalization => {
                "最初の実際の返答のテンポを合わせるため、会話スタイルを軽く保存します。"
            }
            StartupOnboardingStage::Finish => {
                "ここからすぐ会話できます。MCP、web provider、channels、personalization は必要になった時に続きを出せます。"
            }
        },
        Language::Ru => match stage {
            StartupOnboardingStage::Language => {
                "Сначала выберите язык TUI. Позже config.toml всё ещё можно будет тонко подстроить."
            }
            StartupOnboardingStage::Provider => {
                "Выберите provider, который Loong должен подготовить первым. Готовые локальные учётные данные будут показаны автоматически."
            }
            StartupOnboardingStage::Skills => {
                "Loong может заранее поставить несколько bundled skills. Space выбирает, Enter идёт дальше."
            }
            StartupOnboardingStage::SetupPath => {
                "Решите: оставить shell минимальным сейчас или сразу заглянуть в provider, web search, channels и MCP."
            }
            StartupOnboardingStage::Personalization => {
                "Сохраните лёгкий стиль первого разговора, чтобы первый реальный ответ попал в нужный ритм."
            }
            StartupOnboardingStage::Finish => {
                "Теперь можно сразу начать чат; MCP, web provider, channels и personalization можно продолжить по мере надобности."
            }
        },
        _ => match stage {
            StartupOnboardingStage::Language => {
                "choose the TUI language first. You can still fine-tune config.toml later."
            }
            StartupOnboardingStage::Provider => {
                "pick the provider Loong should prepare first. Ready local credentials are surfaced automatically."
            }
            StartupOnboardingStage::Skills => {
                "Loong can preinstall a few bundled skills. Space toggles selection; Enter moves on."
            }
            StartupOnboardingStage::SetupPath => {
                "keep the shell minimal or keep going into the current provider, web search, channels, MCP, and workspace setup details before the first real turn."
            }
            StartupOnboardingStage::Personalization => {
                "save a light first-conversation style so the first real answer lands with the right density and initiative."
            }
            StartupOnboardingStage::Finish => {
                "skip the rest for now. Loong will guide MCP, web-provider setup, and first-turn personalization when a conversation actually needs it."
            }
        },
    }
}

fn startup_onboarding_subtitle_for_state(state: &StartupOnboardingState) -> &'static str {
    if state.stage != StartupOnboardingStage::Finish {
        return startup_onboarding_subtitle(state.stage, state.current_language());
    }

    match (state.current_language(), state.selected_personalization) {
        (Language::ZhCn, Some(StartupPersonalizationPreset::Later)) => {
            "现在可以直接开聊；如果之后想补首轮偏好，再手动运行 `loong personalize`。"
        }
        (Language::ZhCn, Some(StartupPersonalizationPreset::TurnOff)) => {
            "现在可以直接开聊；Loong 不会再主动弹个性化提示，后面需要时仍可手动运行 `loong personalize`。"
        }
        (Language::ZhTw, Some(StartupPersonalizationPreset::Later)) => {
            "現在可以直接開聊；如果之後想補首輪偏好，再手動執行 `loong personalize`。"
        }
        (Language::ZhTw, Some(StartupPersonalizationPreset::TurnOff)) => {
            "現在可以直接開聊；Loong 不會再主動跳出個性化提示，之後需要時仍可手動執行 `loong personalize`。"
        }
        (Language::Ja, Some(StartupPersonalizationPreset::Later)) => {
            "ここからすぐ会話できます。最初の好みを後で足したい時だけ `loong personalize` を使います。"
        }
        (Language::Ja, Some(StartupPersonalizationPreset::TurnOff)) => {
            "ここからすぐ会話できます。Loong は今後個性化の案内を自動では出さず、必要なら手動で `loong personalize` を使えます。"
        }
        (Language::Ru, Some(StartupPersonalizationPreset::Later)) => {
            "Теперь можно сразу начать чат; если позже захотите добавить предпочтения первого разговора, вручную запустите `loong personalize`."
        }
        (Language::Ru, Some(StartupPersonalizationPreset::TurnOff)) => {
            "Теперь можно сразу начать чат; Loong больше не будет сам предлагать персонализацию, но при необходимости вы всё ещё можете вручную запустить `loong personalize`."
        }
        (Language::En, Some(StartupPersonalizationPreset::Later)) => {
            "start chatting now; run `loong personalize` later if you want to capture first-conversation preferences."
        }
        (Language::En, Some(StartupPersonalizationPreset::TurnOff)) => {
            "start chatting now; Loong will stop surfacing personalization prompts unless you later run `loong personalize` yourself."
        }
        _ => startup_onboarding_subtitle(state.stage, state.current_language()),
    }
}

fn startup_feedback_prefix(kind: &str, language: Language) -> &'static str {
    match language {
        Language::ZhCn => match kind {
            "language_set" => "语言已设为",
            "provider_saved" => "provider 选择已保存：",
            "skills_none_selected" => "暂时还没有选任何 skill pack。",
            "skills_skipped" => "先跳过 skills；后面需要时 Loong 还能继续引导安装。",
            "setup_chat_now" => "先把更深的配置留到真正需要时再展开。",
            "setup_provider_web" => {
                "provider 与 web search 的后续路径已经整理好了，下一步可以存一个首轮风格。"
            }
            "setup_channels_delivery" => {
                "channels 与 delivery 的后续路径已经整理好了，下一步可以存一个首轮风格。"
            }
            "setup_mcp_skills" => {
                "MCP 与 workspace 的后续路径已经整理好了，下一步可以存一个首轮风格。"
            }
            _ => "",
        },
        Language::ZhTw => match kind {
            "language_set" => "語言已設為",
            "provider_saved" => "provider 選擇已保存：",
            "skills_none_selected" => "暫時還沒有選任何 skill pack。",
            "skills_skipped" => "先略過 skills；之後需要時 Loong 還能繼續引導安裝。",
            "setup_chat_now" => "先把更深的配置留到真正需要時再展開。",
            "setup_provider_web" => {
                "provider 與 web search 的後續路徑已整理好，下一步可以存一個首輪風格。"
            }
            "setup_channels_delivery" => {
                "channels 與 delivery 的後續路徑已整理好，下一步可以存一個首輪風格。"
            }
            "setup_mcp_skills" => "MCP 與 workspace 的後續路徑已整理好，下一步可以存一個首輪風格。",
            _ => "",
        },
        Language::Ja => match kind {
            "language_set" => "言語を設定:",
            "provider_saved" => "provider を保存:",
            "skills_none_selected" => "skill pack はまだ選択していません。",
            "skills_skipped" => {
                "skills はいったん保留です。必要になったら Loong があとで案内します。"
            }
            "setup_chat_now" => "深い設定は、最初の実際の作業で必要になるまで開きません。",
            "setup_provider_web" => {
                "provider と web search の続き方は整理できました。次は最初の会話スタイルを保存できます。"
            }
            "setup_channels_delivery" => {
                "channels と delivery の続き方は整理できました。次は最初の会話スタイルを保存できます。"
            }
            "setup_mcp_skills" => {
                "MCP と workspace の続き方は整理できました。次は最初の会話スタイルを保存できます。"
            }
            _ => "",
        },
        Language::Ru => match kind {
            "language_set" => "язык выбран:",
            "provider_saved" => "provider сохранён:",
            "skills_none_selected" => "пока не выбран ни один skill pack.",
            "skills_skipped" => {
                "skills пока пропущены. Когда понадобится, Loong подскажет установку позже."
            }
            "setup_chat_now" => {
                "глубокую настройку оставляем до момента, когда она понадобится в реальной работе."
            }
            "setup_provider_web" => {
                "маршрут для provider и web search уже разложен. Дальше можно сохранить стиль первого разговора."
            }
            "setup_channels_delivery" => {
                "маршрут для channels и delivery уже разложен. Дальше можно сохранить стиль первого разговора."
            }
            "setup_mcp_skills" => {
                "маршрут для MCP и workspace уже разложен. Дальше можно сохранить стиль первого разговора."
            }
            _ => "",
        },
        _ => match kind {
            "language_set" => "language set to",
            "provider_saved" => "provider choice saved:",
            "skills_none_selected" => "no skill packs selected yet.",
            "skills_skipped" => "skills skipped for now. Loong can guide installation later.",
            "setup_chat_now" => "keeping deeper setup deferred until the first real task needs it.",
            "setup_provider_web" => {
                "provider and web-search follow-up mapped. next step: save a first-chat style."
            }
            "setup_channels_delivery" => {
                "channel and delivery follow-up mapped. next step: save a first-chat style."
            }
            "setup_mcp_skills" => {
                "MCP and workspace follow-up mapped. next step: save a first-chat style."
            }
            _ => "",
        },
    }
}

fn startup_feedback_selected_skill_packs(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("已选 {count} 个 skill pack。"),
        Language::ZhTw => format!("已選 {count} 個 skill pack。"),
        Language::Ja => format!("{count} 個の skill pack を選択しました。"),
        Language::Ru => format!("выбрано {count} skill pack."),
        _ => format!("selected {count} skill pack(s)."),
    }
}

fn startup_feedback_queued_skill_packs(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("已加入 {count} 个 skill pack，后面仍可继续细调。"),
        Language::ZhTw => format!("已加入 {count} 個 skill pack，之後仍可繼續微調。"),
        Language::Ja => {
            format!("{count} 個の skill pack をキューに入れました。あとでまだ微調整できます。")
        }
        Language::Ru => {
            format!("{count} skill pack добавлены в очередь. Позже это можно ещё уточнить.")
        }
        _ => format!("{count} skill pack(s) queued. You can still refine this later."),
    }
}

fn startup_recommended_badge(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "推荐",
        Language::ZhTw => "推薦",
        Language::Ja => "おすすめ",
        Language::Ru => "рекомендуется",
        Language::En => "recommended",
    }
}

fn startup_personalization_footer_detail(language: Language) -> &'static str {
    match language {
        Language::ZhCn => {
            "Loong 会把这项写进 memory.personalization，只有当这套风格应该投射到 Session Profile 时，才会升级 memory.profile。"
        }
        Language::ZhTw => {
            "Loong 會把這項寫進 memory.personalization，只有當這套風格應該投射到 Session Profile 時，才會升級 memory.profile。"
        }
        Language::Ja => {
            "Loong はこれを memory.personalization に保存し、このスタイルを Session Profile に投影すべき時だけ memory.profile を上げます。"
        }
        Language::Ru => {
            "Loong сохраняет это в memory.personalization и повышает memory.profile только когда этот стиль должен проецироваться в Session Profile."
        }
        _ => {
            "Loong saves this into memory.personalization and only upgrades memory.profile when the saved style should project into Session Profile."
        }
    }
}

fn startup_no_preinstalled_skills_text(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "没有预装 skills",
        Language::ZhTw => "沒有預裝 skills",
        Language::Ja => "プリインストール skill なし",
        Language::Ru => "без предустановленных skills",
        Language::En => "no preinstalled skills",
    }
}

fn startup_selected_skill_count_text(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("已选 {count} 项"),
        Language::ZhTw => format!("已選 {count} 項"),
        Language::Ja => format!("{count} 件を選択"),
        Language::Ru => format!("выбрано {count}"),
        Language::En => format!("{count} selected"),
    }
}

fn startup_not_saved_text(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "未保存",
        Language::ZhTw => "未保存",
        Language::Ja => "未保存",
        Language::Ru => "не сохранено",
        Language::En => "not saved",
    }
}

fn startup_summary_label(kind: &str, language: Language) -> &'static str {
    match language {
        Language::ZhCn => match kind {
            "language" => "语言",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "后续路径",
            "personalization" => "个性化",
            _ => "",
        },
        Language::ZhTw => match kind {
            "language" => "語言",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "後續路徑",
            "personalization" => "個性化",
            _ => "",
        },
        Language::Ja => match kind {
            "language" => "言語",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "続きの設定",
            "personalization" => "会話スタイル",
            _ => "",
        },
        Language::Ru => match kind {
            "language" => "язык",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "дальше настроить",
            "personalization" => "стиль",
            _ => "",
        },
        _ => match kind {
            "language" => "language",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "setup path",
            "personalization" => "personalization",
            _ => "",
        },
    }
}

fn startup_finish_prompt(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "按 Enter 关闭引导并开始聊天。",
        Language::ZhTw => "按 Enter 關閉引導並開始聊天。",
        Language::Ja => "Enter でオンボードを閉じて会話を始めます。",
        Language::Ru => "Нажмите Enter, чтобы закрыть онбординг и начать чат.",
        Language::En => "press Enter to close onboarding and start chatting.",
    }
}

fn startup_eye_animation_for_state(state: Option<&StartupOnboardingState>) -> StartupEyeAnimation {
    let Some(state) = state else {
        return StartupEyeAnimation::Ambient;
    };

    let interaction_age = state.last_interaction_at.elapsed();
    let fresh_navigate = interaction_age < Duration::from_millis(380)
        && state.last_interaction_kind == StartupOnboardingInteractionKind::Navigate;
    let fresh_confirm = interaction_age < Duration::from_millis(640)
        && matches!(
            state.last_interaction_kind,
            StartupOnboardingInteractionKind::Confirm | StartupOnboardingInteractionKind::Persist
        );

    match state.stage {
        StartupOnboardingStage::Language => {
            let focus = if state.language_index == 0 {
                StartupEyeFocus::DownLeft
            } else {
                StartupEyeFocus::DownRight
            };
            if fresh_navigate {
                StartupEyeAnimation::Thinking(focus)
            } else {
                StartupEyeAnimation::Focus(focus)
            }
        }
        StartupOnboardingStage::Provider => {
            let focus = startup_list_focus(state.provider_index, state.provider_options.len());
            if fresh_confirm {
                StartupEyeAnimation::Confirm(focus)
            } else if fresh_navigate {
                StartupEyeAnimation::Thinking(focus)
            } else {
                StartupEyeAnimation::Focus(focus)
            }
        }
        StartupOnboardingStage::Skills => {
            let focus = startup_list_focus(state.skill_cursor, state.skill_options.len());
            if fresh_confirm {
                StartupEyeAnimation::Confirm(focus)
            } else if !state.selected_skill_ids.is_empty() || fresh_navigate {
                StartupEyeAnimation::Thinking(focus)
            } else {
                StartupEyeAnimation::Focus(focus)
            }
        }
        StartupOnboardingStage::SetupPath => match state.current_setup_path_choice() {
            StartupSetupPathChoice::ChatNow => {
                let focus = StartupEyeFocus::DownCenter;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupSetupPathChoice::ProviderAndWeb => {
                let focus = StartupEyeFocus::Right;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else {
                    StartupEyeAnimation::Thinking(focus)
                }
            }
            StartupSetupPathChoice::ChannelsAndDelivery => {
                let focus = StartupEyeFocus::Up;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else {
                    StartupEyeAnimation::Thinking(focus)
                }
            }
            StartupSetupPathChoice::McpAndSkills => {
                let focus = StartupEyeFocus::Left;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else {
                    StartupEyeAnimation::Thinking(focus)
                }
            }
        },
        StartupOnboardingStage::Personalization => match state.current_personalization_preset() {
            StartupPersonalizationPreset::Balanced => {
                let focus = StartupEyeFocus::DownCenter;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::Concise => {
                let focus = StartupEyeFocus::Left;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::Thorough => {
                let focus = StartupEyeFocus::Right;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::Later => {
                let focus = StartupEyeFocus::Up;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::TurnOff => {
                let focus = StartupEyeFocus::Up;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
        },
        StartupOnboardingStage::Finish => StartupEyeAnimation::Celebrate,
    }
}

fn startup_list_focus(index: usize, total: usize) -> StartupEyeFocus {
    if total <= 1 {
        return StartupEyeFocus::DownCenter;
    }

    if index == 0 {
        StartupEyeFocus::DownLeft
    } else if index + 1 >= total {
        StartupEyeFocus::DownRight
    } else {
        StartupEyeFocus::DownCenter
    }
}

fn build_startup_onboarding_footer_line(
    state: &StartupOnboardingState,
    width: u16,
) -> Line<'static> {
    let text = startup_onboarding_footer_text_for_language(state.stage, state.current_language());
    Line::from(Span::styled(
        truncate_right_for_width(text, width as usize),
        Style::default().fg(SURFACE_GRAY),
    ))
}

fn render_startup_onboarding_lines(
    state: &StartupOnboardingState,
    width: u16,
) -> Vec<Line<'static>> {
    let content_width = width.max(24) as usize;
    let mut lines = Vec::new();
    let title = format!(
        "onboarding · {}/{} · {}",
        state.stage.step_index(),
        StartupOnboardingStage::total_steps(),
        state.stage.title(state.current_language())
    );
    lines.push(Line::from(Span::styled(
        truncate_right_for_width(title.as_str(), content_width),
        Style::default()
            .fg(SURFACE_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));

    let subtitle = startup_onboarding_subtitle_for_state(state);
    lines.extend(render_onboarding_wrapped_line(
        "  ",
        subtitle,
        Style::default().fg(SURFACE_GRAY),
        Style::default().fg(SURFACE_GRAY),
        content_width,
    ));

    if let Some(feedback) = state.feedback.as_deref() {
        lines.push(Line::from(""));
        lines.extend(render_onboarding_wrapped_line(
            "✓ ",
            feedback,
            Style::default()
                .fg(SURFACE_GREEN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_GREEN),
            content_width,
        ));
    }

    lines.push(Line::from(""));
    match state.stage {
        StartupOnboardingStage::Language => {
            for (index, language) in state.language_options.iter().enumerate() {
                let selected = index == state.language_index;
                let label = startup_language_label(*language);
                lines.extend(render_onboarding_option_line(
                    selected,
                    label,
                    if *language == Language::En {
                        Some(startup_recommended_badge(state.current_language()))
                    } else {
                        None
                    },
                    content_width,
                ));
            }
        }
        StartupOnboardingStage::Provider => {
            for (index, option) in state.provider_options.iter().enumerate() {
                let selected = index == state.provider_index;
                lines.extend(render_onboarding_option_line(
                    selected,
                    option.label.as_str(),
                    option
                        .recommended
                        .then_some(startup_recommended_badge(state.current_language())),
                    content_width,
                ));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        option.detail.as_str(),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }
        }
        StartupOnboardingStage::Skills => {
            for (index, option) in state.skill_options.iter().enumerate() {
                let selected = index == state.skill_cursor;
                let checked = state
                    .selected_skill_ids
                    .contains(option.install_id.as_str());
                let cursor = if selected { "›" } else { " " };
                let mark = if checked { "[x]" } else { "[ ]" };
                let badge = option
                    .recommended
                    .then_some(startup_recommended_badge(state.current_language()));
                let label = match badge {
                    Some(badge) => format!("{cursor} {mark} {} · {badge}", option.display_name),
                    None => format!("{cursor} {mark} {}", option.display_name),
                };
                lines.push(Line::from(Span::styled(
                    truncate_right_for_width(label.as_str(), content_width),
                    if selected {
                        Style::default()
                            .fg(SURFACE_CYAN)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                )));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        option.summary.as_str(),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }
        }
        StartupOnboardingStage::SetupPath => {
            for (index, choice) in StartupSetupPathChoice::ALL.iter().enumerate() {
                let selected = index == state.setup_path_index;
                lines.extend(render_onboarding_option_line(
                    selected,
                    choice.label(state.current_language()),
                    matches!(choice, StartupSetupPathChoice::ChatNow)
                        .then_some(startup_recommended_badge(state.current_language())),
                    content_width,
                ));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        choice.detail(state.current_language()),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }

            lines.push(Line::from(""));
            for detail in startup_setup_path_detail_lines(state) {
                lines.extend(render_onboarding_wrapped_line(
                    "  • ",
                    detail.as_str(),
                    Style::default().fg(SURFACE_ACCENT),
                    Style::default().fg(Color::White),
                    content_width,
                ));
            }
        }
        StartupOnboardingStage::Personalization => {
            for (index, preset) in StartupPersonalizationPreset::ALL.iter().enumerate() {
                let selected = index == state.personalization_index;
                lines.extend(render_onboarding_option_line(
                    selected,
                    preset.label(state.current_language()),
                    matches!(preset, StartupPersonalizationPreset::Balanced)
                        .then_some(startup_recommended_badge(state.current_language())),
                    content_width,
                ));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        preset.detail(state.current_language()),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }

            lines.push(Line::from(""));
            lines.extend(render_onboarding_wrapped_line(
                "  ",
                startup_personalization_footer_detail(state.current_language()),
                Style::default().fg(SURFACE_GRAY),
                Style::default().fg(SURFACE_GRAY),
                content_width,
            ));
        }
        StartupOnboardingStage::Finish => {
            let language = startup_language_label(state.current_language());
            let provider = state
                .provider_options
                .get(state.provider_index)
                .map(|option| option.label.as_str())
                .unwrap_or("start fresh");
            let skills = if state.selected_skill_ids.is_empty() {
                startup_no_preinstalled_skills_text(state.current_language()).to_owned()
            } else {
                startup_selected_skill_count_text(
                    state.current_language(),
                    state.selected_skill_ids.len(),
                )
            };
            let setup_path = state
                .current_setup_path_choice()
                .label(state.current_language());
            let personalization = state
                .selected_personalization
                .map(|preset| preset.label(state.current_language()))
                .unwrap_or(startup_not_saved_text(state.current_language()));
            for summary in [
                format!(
                    "{} · {language}",
                    startup_summary_label("language", state.current_language())
                ),
                format!(
                    "{} · {provider}",
                    startup_summary_label("provider", state.current_language())
                ),
                format!(
                    "{} · {skills}",
                    startup_summary_label("skills", state.current_language())
                ),
                format!(
                    "{} · {setup_path}",
                    startup_summary_label("setup_path", state.current_language())
                ),
                format!(
                    "{} · {personalization}",
                    startup_summary_label("personalization", state.current_language())
                ),
            ] {
                lines.extend(render_onboarding_wrapped_line(
                    "  • ",
                    summary.as_str(),
                    Style::default().fg(SURFACE_ACCENT),
                    Style::default().fg(Color::White),
                    content_width,
                ));
            }
            lines.push(Line::from(""));
            lines.extend(render_onboarding_wrapped_line(
                "  ",
                startup_finish_prompt(state.current_language()),
                Style::default().fg(SURFACE_GRAY),
                Style::default().fg(SURFACE_GRAY),
                content_width,
            ));
        }
    }

    lines
}

fn startup_setup_path_detail_lines(state: &StartupOnboardingState) -> Vec<String> {
    let language = state.current_language();
    match state.current_setup_path_choice() {
        StartupSetupPathChoice::ChatNow => match language {
            Language::ZhCn => vec![
                "当前 splash/chat shell 会保持不变；更深的配置随时都能按需展开。".to_owned(),
                "如果你之后想走完整的 provider、web、channel、daemon 向导，可以再用 `loong onboard`。".to_owned(),
                "如果你想先看当前 runtime snapshot，也可以随时在空白输入框里用 /mcp 或 /skills。".to_owned(),
            ],
            Language::ZhTw => vec![
                "目前 splash/chat shell 會保持不變；更深的配置隨時都能按需展開。".to_owned(),
                "如果你之後想走完整的 provider、web、channel、daemon 嚮導，可以再用 `loong onboard`。".to_owned(),
                "如果你想先看目前 runtime snapshot，也可以隨時在空白輸入框裡用 /mcp 或 /skills。".to_owned(),
            ],
            Language::En | Language::Ja | Language::Ru => vec![
                "The current splash/chat shell stays intact; deeper setup remains available on demand."
                    .to_owned(),
                "Use `loong onboard` later when you want the full provider, web, channel, and daemon wizard."
                    .to_owned(),
                "Use /mcp or /skills from the empty composer whenever you want the live runtime snapshot."
                    .to_owned(),
            ],
        },
        StartupSetupPathChoice::ProviderAndWeb => match language {
            Language::ZhCn => vec![
                format!(
                    "当前 provider 路径：{}。",
                    state
                        .provider_options
                        .get(state.provider_index)
                        .map(|option| option.label.as_str())
                        .unwrap_or("provider")
                ),
                match state.provider_auth_env_name.as_deref() {
                    Some(env_name) => format!("当前 provider 凭证环境变量：{}。", env_name),
                    None => "当前 provider 还没有解析到可用的凭证环境变量。".to_owned(),
                },
                format!("当前 web 默认项：{}。", state.web_search_provider_label),
                state.web_search_provider_detail.clone(),
                state.provider_configuration_hint.clone().unwrap_or_else(|| {
                    "如果你要继续补 provider 的 base_url、model 或鉴权，下一步优先看 `loong doctor`。"
                        .to_owned()
                }),
                "完整的 provider / auth continuation 仍然在 `loong onboard`；这里会保持 shell 轻一点，只把真正的下一条命令露出来。"
                    .to_owned(),
            ],
            Language::ZhTw => vec![
                format!(
                    "目前 provider 路徑：{}。",
                    state
                        .provider_options
                        .get(state.provider_index)
                        .map(|option| option.label.as_str())
                        .unwrap_or("provider")
                ),
                match state.provider_auth_env_name.as_deref() {
                    Some(env_name) => format!("目前 provider 憑證環境變數：{}。", env_name),
                    None => "目前 provider 還沒有解析到可用的憑證環境變數。".to_owned(),
                },
                format!("目前 web 預設項：{}。", state.web_search_provider_label),
                state.web_search_provider_detail.clone(),
                state.provider_configuration_hint.clone().unwrap_or_else(|| {
                    "如果你要繼續補 provider 的 base_url、model 或驗證，下一步優先看 `loong doctor`。"
                        .to_owned()
                }),
                "完整的 provider / auth continuation 仍然在 `loong onboard`；這裡會保持 shell 輕一點，只把真正的下一條命令露出來。"
                    .to_owned(),
            ],
            Language::En | Language::Ja | Language::Ru => vec![
                format!(
                    "Provider lane now: {}.",
                    state
                        .provider_options
                        .get(state.provider_index)
                        .map(|option| option.label.as_str())
                        .unwrap_or("provider")
                ),
                match state.provider_auth_env_name.as_deref() {
                    Some(env_name) => format!("Provider auth env now: {}.", env_name),
                    None => "No provider credential env is resolved yet.".to_owned(),
                },
                format!("Web setup default: {}.", state.web_search_provider_label),
                state.web_search_provider_detail.clone(),
                state.provider_configuration_hint.clone().unwrap_or_else(|| {
                    "If you need to keep tuning provider base_url, model, or auth, `loong doctor` is the next check to run."
                        .to_owned()
                }),
                "Full provider/auth continuation lives in `loong onboard`; the TUI keeps the shell minimal and surfaces the real next command instead of opening a second startup UI."
                    .to_owned(),
            ],
        },
        StartupSetupPathChoice::ChannelsAndDelivery => {
            let has_enabled_channels = !state.enabled_channel_labels.is_empty();
            let suggested_channels = startup_suggested_channel_follow_up_descriptors(language);
            let mut lines = vec![match language {
                Language::ZhCn if state.enabled_channel_labels.is_empty() => {
                    "当前还没有启用任何外部 channel。".to_owned()
                }
                Language::ZhTw if state.enabled_channel_labels.is_empty() => {
                    "目前還沒有啟用任何外部 channel。".to_owned()
                }
                Language::ZhCn => format!(
                    "当前已启用的 channel：{}。",
                    state.enabled_channel_labels.join(", ")
                ),
                Language::ZhTw => format!(
                    "目前已啟用的 channel：{}。",
                    state.enabled_channel_labels.join(", ")
                ),
                Language::En | Language::Ja | Language::Ru
                    if state.enabled_channel_labels.is_empty() =>
                {
                    "No external channels are enabled yet.".to_owned()
                }
                Language::En | Language::Ja | Language::Ru => {
                    format!("Enabled channels now: {}.", state.enabled_channel_labels.join(", "))
                }
            }];
            if !has_enabled_channels && !suggested_channels.is_empty() {
                let suggested_labels = suggested_channels
                    .iter()
                    .map(|descriptor| descriptor.label.clone())
                    .collect::<Vec<_>>();
                lines.push(match language {
                    Language::ZhCn => {
                        format!("建议优先接的 channels：{}。", suggested_labels.join(", "))
                    }
                    Language::ZhTw => {
                        format!("建議優先接的 channels：{}。", suggested_labels.join(", "))
                    }
                    Language::Ja => {
                        format!("まず候補になる channels: {}。", suggested_labels.join(", "))
                    }
                    Language::Ru => {
                        format!("С чего обычно начинают: {}.", suggested_labels.join(", "))
                    }
                    Language::En => {
                        format!("Good first channels to wire: {}.", suggested_labels.join(", "))
                    }
                });

                let suggested_commands = suggested_channels
                    .iter()
                    .filter_map(|descriptor| descriptor.repair_command.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if !suggested_commands.is_empty() {
                    lines.push(match language {
                        Language::ZhCn => {
                            format!("建议先跑的 onboarding 命令：{}。", suggested_commands.join(", "))
                        }
                        Language::ZhTw => {
                            format!("建議先跑的 onboarding 命令：{}。", suggested_commands.join(", "))
                        }
                        Language::Ja => {
                            format!("先に試す onboarding コマンド: {}。", suggested_commands.join(", "))
                        }
                        Language::Ru => {
                            format!("Сначала стоит запустить: {}.", suggested_commands.join(", "))
                        }
                        Language::En => {
                            format!("Suggested onboarding commands: {}.", suggested_commands.join(", "))
                        }
                    });
                }
            }
            if state.channel_follow_up_commands.is_empty() {
                lines.push(match language {
                    Language::ZhCn => {
                        "当前还没有可直接运行的 channel runtime command；如果你要继续接 delivery surface，可以走 `loong onboard`。".to_owned()
                    }
                    Language::ZhTw => {
                        "目前還沒有可直接執行的 channel runtime command；如果你要繼續接 delivery surface，可以走 `loong onboard`。".to_owned()
                    }
                    Language::En | Language::Ja | Language::Ru => {
                        "No channel runtime command is ready yet; continue setup through `loong onboard` when you want to wire delivery surfaces.".to_owned()
                    }
                });
            } else if let Some(next_command) = state.channel_follow_up_commands.first() {
                lines.push(match language {
                    Language::ZhCn => format!("下一条 runtime command：{}。", next_command),
                    Language::ZhTw => format!("下一條 runtime command：{}。", next_command),
                    Language::En | Language::Ja | Language::Ru => {
                        format!("Next runtime command: {}.", next_command)
                    }
                });
                if let Some(other_commands) = state.channel_follow_up_commands.get(1..)
                    && !other_commands.is_empty()
                {
                    lines.push(match language {
                        Language::ZhCn => format!(
                            "其他 channel command：{}。",
                            other_commands.join(", ")
                        ),
                        Language::ZhTw => format!(
                            "其他 channel command：{}。",
                            other_commands.join(", ")
                        ),
                        Language::En | Language::Ja | Language::Ru => format!(
                            "Other channel commands: {}.",
                            other_commands.join(", ")
                        ),
                    });
                }
            }
            if let Some(status_command) = state.channel_status_commands.first() {
                lines.push(match language {
                    Language::ZhCn => format!("健康检查命令：{}。", status_command),
                    Language::ZhTw => format!("健康檢查命令：{}。", status_command),
                    Language::Ja => format!("ヘルス確認コマンド: {}。", status_command),
                    Language::Ru => format!("Команда проверки состояния: {}.", status_command),
                    Language::En => format!("Health command: {}.", status_command),
                });
            }
            if !state.channel_repair_commands.is_empty() {
                lines.push(match language {
                    Language::ZhCn => format!(
                        "修复路径：{}。",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::ZhTw => format!(
                        "修復路徑：{}。",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::Ja => format!(
                        "修復パス: {}。",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::Ru => format!(
                        "Путь восстановления: {}.",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::En => format!(
                        "Repair path: {}.",
                        state.channel_repair_commands.join(", ")
                    ),
                });
            }
            lines.push(match language {
                Language::ZhCn => {
                    "这条路径会先保持当前聊天面简洁，但把你下一步真正要继续的 channel / delivery 命令露出来。".to_owned()
                }
                Language::ZhTw => {
                    "這條路徑會先保持目前聊天面精簡，但把你下一步真正要繼續的 channel / delivery 命令露出來。".to_owned()
                }
                Language::En | Language::Ja | Language::Ru => {
                    "This path keeps chat focused now while surfacing the real channel and delivery commands you can continue with next.".to_owned()
                }
            });
            lines
        }
        StartupSetupPathChoice::McpAndSkills => match language {
            Language::ZhCn => vec![
                format!("当前可用的 bootstrap MCP server：{}。", state.startup_mcp_count),
                format!(
                    "当前可见的 bundled skill pack：{}（这轮已选 {} 个）。",
                    state.detected_skill_count,
                    state.selected_skill_ids.len()
                ),
                "如果你想看 live server list 用 /mcp；想看 pack inventory 用 /skills；想走更完整的 workspace/setup wizard 还是用 `loong onboard`。".to_owned(),
            ],
            Language::ZhTw => vec![
                format!("目前可用的 bootstrap MCP server：{}。", state.startup_mcp_count),
                format!(
                    "目前可見的 bundled skill pack：{}（這輪已選 {} 個）。",
                    state.detected_skill_count,
                    state.selected_skill_ids.len()
                ),
                "如果你想看 live server list 用 /mcp；想看 pack inventory 用 /skills；想走更完整的 workspace/setup wizard 還是用 `loong onboard`。".to_owned(),
            ],
            Language::En | Language::Ja | Language::Ru => vec![
                format!("Bootstrap MCP servers available now: {}.", state.startup_mcp_count),
                format!(
                    "Bundled skill packs visible now: {} ({} selected in this startup pass).",
                    state.detected_skill_count,
                    state.selected_skill_ids.len()
                ),
                "Use /mcp for the live server list, /skills for the pack inventory, or `loong onboard` if you want the broader workspace/setup wizard."
                    .to_owned(),
            ],
        },
    }
}

fn startup_web_search_detail(
    runtime: &CliTurnRuntime,
    provider: &str,
    language: Language,
) -> String {
    if runtime
        .config
        .tools
        .web_search
        .configured_api_key_for_provider(provider)
        .is_some()
    {
        let provider_label = web_search_provider_descriptor(provider)
            .map(|descriptor| descriptor.display_name)
            .unwrap_or(provider);
        return match language {
            Language::ZhCn => {
                format!(
                    "web provider 已就绪：{} 已在 tools.web_search 里配置好。",
                    provider_label
                )
            }
            Language::ZhTw => {
                format!(
                    "web provider 已就緒：{} 已在 tools.web_search 裡配置好。",
                    provider_label
                )
            }
            Language::Ja => format!(
                "web provider は準備できています: {} はすでに tools.web_search に設定されています。",
                provider_label
            ),
            Language::Ru => format!(
                "web provider готов: {} уже настроен в tools.web_search.",
                provider_label
            ),
            Language::En => format!(
                "Web provider ready: {} is already configured inside tools.web_search.",
                provider_label
            ),
        };
    }

    if let Some(env_name) = web_search_provider_api_key_env_names(provider)
        .iter()
        .find(|env_name| std::env::var_os(env_name).is_some())
    {
        let provider_label = web_search_provider_descriptor(provider)
            .map(|descriptor| descriptor.display_name)
            .unwrap_or(provider);
        return match language {
            Language::ZhCn => {
                format!("web provider 后续可以复用 {env_name}：{}。", provider_label)
            }
            Language::ZhTw => {
                format!("web provider 後續可以重用 {env_name}：{}。", provider_label)
            }
            Language::Ja => format!(
                "後で setup を続けるなら、web provider {} は {env_name} を再利用できます。",
                provider_label
            ),
            Language::Ru => format!(
                "Если потом продолжить setup, web provider {} сможет переиспользовать {env_name}.",
                provider_label
            ),
            Language::En => format!(
                "Web provider follow-up: {} can reuse {env_name} if you continue setup later.",
                provider_label
            ),
        };
    }

    let provider_label = web_search_provider_descriptor(provider)
        .map(|descriptor| descriptor.display_name)
        .unwrap_or(provider);
    match language {
        Language::ZhCn => format!(
            "web provider 后续默认会走 {}，但要继续 web 相关配置，鉴权还需要补上。",
            provider_label
        ),
        Language::ZhTw => format!(
            "web provider 後續預設會走 {}，但要繼續 web 相關配置，驗證還需要補上。",
            provider_label
        ),
        Language::Ja => format!(
            "web provider の既定は {} ですが、web 連携を先へ進めるには認証の配線がまだ必要です。",
            provider_label
        ),
        Language::Ru => format!(
            "По умолчанию web provider сейчас {}, но чтобы продолжить web-настройку, авторизацию ещё нужно подключить.",
            provider_label
        ),
        Language::En => format!(
            "Web provider follow-up: {} is the current default, but auth still needs to be wired before web-backed setup can go further.",
            provider_label
        ),
    }
}

fn startup_channel_follow_up_descriptors(
    runtime: &CliTurnRuntime,
    language: Language,
) -> Vec<StartupChannelFollowUpDescriptor> {
    service_channel_descriptors()
        .into_iter()
        .filter(|descriptor| channel_enabled_in_config(&runtime.config, descriptor.id))
        .filter_map(|descriptor| {
            let onboarding = resolve_channel_onboarding_descriptor(descriptor.id)?;
            Some(StartupChannelFollowUpDescriptor {
                label: startup_channel_label(descriptor.id, descriptor.label, language),
                serve_command: descriptor.serve_subcommand.map(str::to_owned),
                status_command: onboarding.status_command.to_owned(),
                repair_command: onboarding.repair_command.map(str::to_owned),
            })
        })
        .collect()
}

fn startup_suggested_channel_follow_up_descriptors(
    language: Language,
) -> Vec<StartupChannelFollowUpDescriptor> {
    preferred_startup_channel_ids(language)
        .iter()
        .filter_map(|channel_id| {
            let descriptor = service_channel_descriptors()
                .into_iter()
                .find(|descriptor| descriptor.id == *channel_id)?;
            let onboarding = resolve_channel_onboarding_descriptor(descriptor.id)?;
            Some(StartupChannelFollowUpDescriptor {
                label: startup_channel_label(descriptor.id, descriptor.label, language),
                serve_command: descriptor.serve_subcommand.map(str::to_owned),
                status_command: onboarding.status_command.to_owned(),
                repair_command: onboarding.repair_command.map(str::to_owned),
            })
        })
        .collect()
}

fn preferred_startup_channel_ids(language: Language) -> &'static [&'static str] {
    match language {
        Language::ZhCn | Language::ZhTw => &["feishu", "wecom", "dingtalk", "weixin"],
        Language::Ja => &["line", "telegram", "discord", "slack"],
        Language::Ru => &["telegram", "matrix", "discord", "slack"],
        Language::En => &["telegram", "matrix", "slack", "discord"],
    }
}

fn channel_enabled_in_config(config: &LoongConfig, channel_id: &str) -> bool {
    match channel_id {
        "cli" => config.cli.enabled,
        "telegram" => config.telegram.enabled,
        "feishu" => config.feishu.enabled,
        "matrix" => config.matrix.enabled,
        "wecom" => config.wecom.enabled,
        "weixin" => config.weixin.enabled,
        "qqbot" => config.qqbot.enabled,
        "onebot" => config.onebot.enabled,
        "discord" => config.discord.enabled,
        "line" => config.line.enabled,
        "dingtalk" => config.dingtalk.enabled,
        "webhook" => config.webhook.enabled,
        "slack" => config.slack.enabled,
        "google-chat" => config.google_chat.enabled,
        "mattermost" => config.mattermost.enabled,
        "nextcloud-talk" => config.nextcloud_talk.enabled,
        "synology-chat" => config.synology_chat.enabled,
        "irc" => config.irc.enabled,
        "imessage" => config.imessage.enabled,
        "signal" => config.signal.enabled,
        "whatsapp" => config.whatsapp.enabled,
        "teams" => config.teams.enabled,
        "twitch" => config.twitch.enabled,
        "nostr" => config.nostr.enabled,
        "tlon" => config.tlon.enabled,
        _ => false,
    }
}

fn startup_channel_label(channel_id: &str, fallback_label: &str, language: Language) -> String {
    match language {
        Language::ZhCn => match channel_id {
            "feishu" => "飞书".to_owned(),
            "dingtalk" => "钉钉".to_owned(),
            "wecom" => "企业微信".to_owned(),
            "weixin" => "微信".to_owned(),
            _ => fallback_label.to_owned(),
        },
        Language::ZhTw => match channel_id {
            "feishu" => "飛書".to_owned(),
            "dingtalk" => "釘釘".to_owned(),
            "wecom" => "企業微信".to_owned(),
            "weixin" => "微信".to_owned(),
            _ => fallback_label.to_owned(),
        },
        Language::En | Language::Ja | Language::Ru => fallback_label.to_owned(),
    }
}

fn startup_personalization_locale(language: Language) -> &'static str {
    match language {
        Language::En => "en-US",
        Language::ZhCn => "zh-CN",
        Language::ZhTw => "zh-TW",
        Language::Ja => "ja-JP",
        Language::Ru => "ru-RU",
    }
}

fn startup_first_turn_bootstrap_addendum(
    preset: StartupPersonalizationPreset,
    language: Language,
) -> Option<String> {
    if matches!(
        preset,
        StartupPersonalizationPreset::Later | StartupPersonalizationPreset::TurnOff
    ) {
        return None;
    }

    let instruction = match (preset, language) {
        (StartupPersonalizationPreset::Concise, Language::ZhCn) => concat!(
            "仅在下一次真正回复里生效：先不要直接解决任务。先用一条非常简短、自然、友善的 assistant 消息确认该怎么称呼用户；",
            "如果语气允许，再顺带问一句 Loong 该叫什么。不要列清单，不要提到系统提示，问完就停下来等用户回答。"
        ),
        (StartupPersonalizationPreset::Concise, Language::ZhTw) => concat!(
            "僅在下一次真正回覆裡生效：先不要直接解決任務。先用一條非常簡短、自然、友善的 assistant 訊息確認該怎麼稱呼使用者；",
            "如果語氣允許，再順帶問一句 Loong 該叫什麼。不要列清單，不要提到系統提示，問完就停下來等使用者回答。"
        ),
        (StartupPersonalizationPreset::Concise, Language::Ja) => concat!(
            "次の実際の返信で一度だけ使います。いきなり作業を進めず、まず短く自然でやわらかい assistant メッセージを一つ送り、",
            "ユーザーをどう呼べばよいかを確認してください。流れが自然なら、Loong の呼び名も一言だけ聞いてください。箇条書きや system prompt への言及は避け、聞いたら止まって返答を待ちます。"
        ),
        (StartupPersonalizationPreset::Concise, Language::Ru) => concat!(
            "Это действует только для следующего реального ответа. Не переходи сразу к задаче.",
            " Сначала отправь очень короткое, естественное и дружелюбное сообщение assistant и уточни, как обращаться к пользователю.",
            " Если это уместно, одной фразой спроси и как лучше звать Loong. Без чеклиста и без упоминания system prompt; после вопроса остановись и жди ответа."
        ),
        (StartupPersonalizationPreset::Concise, Language::En) => concat!(
            "For the next real reply only, do not jump straight into the task.",
            " First send one very short, natural, friendly assistant message that confirms how to address the user.",
            " If it feels natural, ask in one clause what Loong should be called too. No checklist, no system-prompt talk, and stop after asking."
        ),
        (StartupPersonalizationPreset::Thorough, Language::ZhCn) => concat!(
            "仅在下一次真正回复里生效：先不要直接解决任务。先用一条自然、简短、友善的 assistant 消息确认：",
            "该怎么称呼用户；如果用户愿意，Loong 应该叫什么、整体气质想偏什么感觉、要不要带一个 emoji；如果和协作有关，也可以顺带说一下时区、长期边界，或者你想让 Loong 记住的一条备注。",
            "不要写成清单，不要提到系统提示或个性化流程，问完后停下来等待用户回答。"
        ),
        (StartupPersonalizationPreset::Thorough, Language::ZhTw) => concat!(
            "僅在下一次真正回覆裡生效：先不要直接解決任務。先用一條自然、簡短、友善的 assistant 訊息確認：",
            "該怎麼稱呼使用者；如果使用者願意，Loong 應該叫什麼、整體氣質想偏什麼感覺、要不要帶一個 emoji；如果和協作有關，也可以順帶說一下時區、長期邊界，或是一條希望 Loong 記住的備註。",
            "不要寫成清單，不要提到系統提示或個性化流程，問完後停下來等待使用者回答。"
        ),
        (StartupPersonalizationPreset::Thorough, Language::Ja) => concat!(
            "次の実際の返信で一度だけ使います。いきなり作業を進めず、まず短く自然でやわらかい assistant メッセージを一つ送り、",
            "どう呼べばよいかを確認してください。望むなら Loong の呼び名、雰囲気、emoji の好みも一緒に聞いて構いません。協力に関係するなら、タイムゾーンや長く保ちたい境界、覚えておいてほしい短いメモも軽く聞いて構いません。",
            "箇条書きや system prompt への言及は避け、聞いたら止まって返答を待ちます。"
        ),
        (StartupPersonalizationPreset::Thorough, Language::Ru) => concat!(
            "Это действует только для следующего реального ответа. Не переходи сразу к задаче.",
            " Сначала отправь одно короткое, естественное и дружелюбное сообщение assistant и мягко уточни,",
            " как обращаться к пользователю; если ему хочется, как звать Loong, какой vibe ближе и нужен ли emoji. Если это помогает совместной работе, можно заодно спросить про timezone, постоянные boundaries или короткую заметку, которую Loong стоит помнить.",
            " Не оформляй это как чеклист и не упоминай system prompt или personalization workflow; после вопроса остановись и жди ответа."
        ),
        (StartupPersonalizationPreset::Thorough, Language::En) => concat!(
            "For the next real reply only, do not jump straight into the task.",
            " First send one short, natural, friendly assistant message that asks how to address the user and, if they want, what name, vibe, or emoji Loong should use. If it helps future collaboration, you may also invite a timezone, a long-lived boundary, or one short note Loong should remember.",
            " Do not present a checklist, do not mention system prompts or personalization workflow, and stop after asking so the user can answer."
        ),
        (_, Language::ZhCn) => concat!(
            "仅在下一次真正回复里生效：先不要直接解决任务。先用一条自然、简短、友善的 assistant 消息问清两件事：",
            "1）该怎么称呼用户；2）如果用户愿意，Loong 应该叫什么、整体气质/emoji 想要什么感觉。",
            "不要把问题写成清单，不要提到“系统提示”或“个性化流程”，也不要一次问太多。问完后停下来等待用户回答。"
        ),
        (_, Language::ZhTw) => concat!(
            "僅在下一次真正回覆裡生效：先不要直接解決任務。先用一條自然、簡短、友善的 assistant 訊息問清兩件事：",
            "1）該怎麼稱呼使用者；2）如果使用者願意，Loong 應該叫什麼、整體氣質/emoji 想要什麼感覺。",
            "不要把問題寫成清單，不要提到「系統提示」或「個性化流程」，也不要一次問太多。問完後停下來等待使用者回答。"
        ),
        (_, Language::Ja) => concat!(
            "次の実際の返信で一度だけ使います。いきなり作業を進めず、まず短く自然でやわらかい assistant メッセージを一つ送り、",
            "1) ユーザーをどう呼べばよいか、2) 望むなら Loong の呼び名や雰囲気、emoji の好みも教えてほしい、と聞いてください。",
            "箇条書きにはせず、system prompt や personalization workflow には触れず、聞き終えたら返答を待って止まってください。"
        ),
        (_, Language::Ru) => concat!(
            "Это действует только для следующего реального ответа. Не переходи сразу к задаче.",
            " Сначала отправь одно короткое, естественное и дружелюбное сообщение assistant, где мягко уточнишь:",
            " 1) как обращаться к пользователю; 2) если пользователю хочется, как звать Loong и какой vibe или emoji ему ближе.",
            " Не оформляй это как чеклист, не упоминай system prompt или personalization workflow, и после вопроса остановись в ожидании ответа."
        ),
        (_, Language::En) => concat!(
            "For the next real reply only, do not jump straight into the task.",
            " First send one short, natural, friendly assistant message that asks two things:",
            " how to address the user, and, if they want, what name, vibe, or emoji Loong should use.",
            " Do not present a checklist, do not mention system prompts or personalization workflow, and stop after asking so the user can answer."
        ),
    };

    Some(instruction.to_owned())
}

fn apply_first_turn_bootstrap_addendum(runtime: &mut CliTurnRuntime, addendum: String) {
    if addendum.trim().is_empty() {
        return;
    }

    runtime.config.cli.system_prompt_addendum = Some(addendum.clone());
    if !runtime.config.cli.uses_native_prompt_pack() {
        let base = runtime.config.cli.system_prompt.trim().to_owned();
        runtime.config.cli.system_prompt = if base.is_empty() {
            addendum
        } else {
            format!("{base}\n\n{addendum}")
        };
    }
}

fn maybe_capture_and_persist_first_turn_bootstrap_reply(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    input: &str,
) -> CliResult<()> {
    if !app.awaiting_first_turn_bootstrap_reply {
        return Ok(());
    }

    if detect_startup_bootstrap_reply_opt_out(input) {
        app.awaiting_first_turn_bootstrap_reply = false;
        return Ok(());
    }

    let Some(capture) = infer_startup_bootstrap_capture(input) else {
        return Ok(());
    };
    app.awaiting_first_turn_bootstrap_reply = false;
    persist_startup_bootstrap_capture(runtime, &capture)
}

fn detect_startup_bootstrap_reply_opt_out(input: &str) -> bool {
    let lowered = input.to_ascii_lowercase();
    [
        "skip for now",
        "skip this",
        "no preference",
        "up to you",
        "either is fine",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern))
        || [
            "跳过",
            "不用了",
            "随便",
            "隨便",
            "都可以",
            "先这样",
            "先這樣",
        ]
        .iter()
        .any(|pattern| input.contains(pattern))
}

fn infer_startup_bootstrap_capture(input: &str) -> Option<StartupBootstrapCapture> {
    let capture = StartupBootstrapCapture {
        preferred_address: extract_bootstrap_field_value(
            input,
            &[
                "you can call me ",
                "call me ",
                "address me as ",
                "叫我",
                "稱呼我",
                "称呼我",
            ],
        ),
        pronouns: extract_bootstrap_field_value(
            input,
            &[
                "my pronouns are ",
                "pronouns are ",
                "pronouns=",
                "代词是",
                "代詞是",
            ],
        ),
        agent_name: extract_bootstrap_field_value(
            input,
            &[
                "your name is ",
                "call yourself ",
                "you can be ",
                "你叫",
                "你可以叫",
                "loong叫",
                "loong 叫",
            ],
        ),
        creature: extract_bootstrap_field_value(
            input,
            &[
                "creature=",
                "creature is ",
                "species is ",
                "物种是",
                "物種是",
                "设定是",
                "設定是",
            ],
        ),
        vibe: extract_bootstrap_field_value(
            input,
            &[
                "vibe=",
                "vibe is ",
                "tone is ",
                "气质是",
                "氣質是",
                "风格是",
                "風格是",
            ],
        ),
        emoji: extract_bootstrap_field_value(
            input,
            &["emoji=", "emoji is ", "emoji 用", "emoji用", "表情是"],
        ),
        timezone: extract_bootstrap_field_value(
            input,
            &[
                "timezone=",
                "timezone is ",
                "my timezone is ",
                "time zone is ",
                "时区是",
                "時區是",
            ],
        ),
        standing_boundaries: extract_bootstrap_field_value(
            input,
            &[
                "boundaries are ",
                "boundary is ",
                "boundary=",
                "standing boundaries are ",
                "边界是",
                "界限是",
                "原則是",
                "原则是",
            ],
        ),
        notes: extract_bootstrap_field_value(
            input,
            &[
                "notes are ",
                "note is ",
                "note: ",
                "notes: ",
                "keep in mind ",
                "备注是",
                "備註是",
                "备注：",
                "備註：",
            ],
        ),
    };

    if capture == StartupBootstrapCapture::default() {
        None
    } else {
        Some(capture)
    }
}

fn extract_bootstrap_field_value(input: &str, patterns: &[&str]) -> Option<String> {
    for pattern in patterns {
        let value = if pattern.is_ascii() {
            extract_ascii_pattern_value(input, pattern)
        } else {
            extract_direct_pattern_value(input, pattern)
        };
        if value.is_some() {
            return value;
        }
    }
    None
}

fn extract_ascii_pattern_value(input: &str, pattern: &str) -> Option<String> {
    let haystack = input.to_ascii_lowercase();
    let index = haystack.find(pattern)?;
    let start = index + pattern.len();
    normalize_bootstrap_value(&input[start..])
}

fn extract_direct_pattern_value(input: &str, pattern: &str) -> Option<String> {
    let index = input.find(pattern)?;
    let start = index + pattern.len();
    normalize_bootstrap_value(&input[start..])
}

fn normalize_bootstrap_value(value: &str) -> Option<String> {
    let trimmed = value
        .trim_start_matches([' ', ':', '：', '=', '-', '—'])
        .trim();
    if trimmed.is_empty() {
        return None;
    }

    let end_index = trimmed
        .char_indices()
        .find_map(|(index, ch)| {
            if matches!(
                ch,
                ';' | '\n' | '。' | '，' | ',' | '.' | '!' | '?' | '！' | '？'
            ) {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(trimmed.len());
    let normalized = trimmed[..end_index]
        .trim()
        .trim_matches([
            '"', '\'', '“', '”', '「', '」', '『', '』', '。', '，', ',', '.',
        ])
        .trim();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.to_owned())
}

fn persist_startup_bootstrap_capture(
    runtime: &mut CliTurnRuntime,
    capture: &StartupBootstrapCapture,
) -> CliResult<()> {
    let mut config = runtime.config.clone();
    let path = runtime.resolved_path.display().to_string();
    let personalization = config
        .memory
        .personalization
        .get_or_insert_with(PersonalizationConfig::default);
    if let Some(preferred_address) = capture.preferred_address.as_deref() {
        personalization.preferred_name = Some(preferred_address.to_owned());
    }
    if let Some(pronouns) = capture.pronouns.as_deref() {
        personalization.pronouns = Some(pronouns.to_owned());
    }
    if let Some(timezone) = capture.timezone.as_deref() {
        personalization.timezone = Some(timezone.to_owned());
    }
    if let Some(standing_boundaries) = capture.standing_boundaries.as_deref() {
        personalization.standing_boundaries = Some(standing_boundaries.to_owned());
    }
    if let Some(notes) = capture.notes.as_deref() {
        personalization.notes = Some(notes.to_owned());
    }
    if personalization.prompt_state == PersonalizationPromptState::Pending
        && (capture.preferred_address.is_some()
            || capture.pronouns.is_some()
            || capture.timezone.is_some()
            || capture.standing_boundaries.is_some()
            || capture.notes.is_some())
    {
        personalization.prompt_state = PersonalizationPromptState::Configured;
    }

    crate::config::write(Some(path.as_str()), &config, true)?;
    runtime.config = config;

    let workspace_root = current_working_directory(runtime);
    persist_startup_bootstrap_runtime_self_files(workspace_root.as_path(), capture)
}

fn persist_startup_bootstrap_runtime_self_files(
    workspace_root: &Path,
    capture: &StartupBootstrapCapture,
) -> CliResult<()> {
    upsert_bootstrap_runtime_self_file(
        workspace_root.join("USER.md").as_path(),
        "# User",
        render_bootstrap_user_block(capture),
    )?;
    upsert_bootstrap_runtime_self_file(
        workspace_root.join("IDENTITY.md").as_path(),
        "# Identity",
        render_bootstrap_identity_block(capture),
    )?;
    upsert_bootstrap_runtime_self_file(
        workspace_root.join("SOUL.md").as_path(),
        "# Soul",
        render_bootstrap_soul_block(capture),
    )?;
    Ok(())
}

fn render_bootstrap_user_block(capture: &StartupBootstrapCapture) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(preferred_address) = capture.preferred_address.as_deref() {
        lines.push(format!("- Preferred address: {preferred_address}"));
    }
    if let Some(pronouns) = capture.pronouns.as_deref() {
        lines.push(format!("- Pronouns: {pronouns}"));
    }
    if let Some(timezone) = capture.timezone.as_deref() {
        lines.push(format!("- Timezone: {timezone}"));
    }
    if let Some(standing_boundaries) = capture.standing_boundaries.as_deref() {
        lines.push(format!("- Standing boundaries: {standing_boundaries}"));
    }
    if let Some(notes) = capture.notes.as_deref() {
        lines.push(format!("- Notes: {notes}"));
    }

    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn render_bootstrap_identity_block(capture: &StartupBootstrapCapture) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(agent_name) = capture.agent_name.as_deref() {
        lines.push(format!("- Name: {agent_name}"));
    }
    if let Some(creature) = capture.creature.as_deref() {
        lines.push(format!("- Creature: {creature}"));
    }
    if let Some(vibe) = capture.vibe.as_deref() {
        lines.push(format!("- Vibe: {vibe}"));
    }
    if let Some(emoji) = capture.emoji.as_deref() {
        lines.push(format!("- Emoji: {emoji}"));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn render_bootstrap_soul_block(capture: &StartupBootstrapCapture) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(vibe) = capture.vibe.as_deref() {
        lines.push(format!("- Preferred vibe: {vibe}"));
    }
    if let Some(emoji) = capture.emoji.as_deref() {
        lines.push(format!("- Signature emoji: {emoji}"));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn upsert_bootstrap_runtime_self_file(
    path: &Path,
    heading: &str,
    block_body: Option<String>,
) -> CliResult<()> {
    let Some(block_body) = block_body else {
        return Ok(());
    };
    let start_marker = "<!-- loong-bootstrap:start -->";
    let end_marker = "<!-- loong-bootstrap:end -->";
    let managed_block = format!("{start_marker}\n{block_body}\n{end_marker}");
    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = if let Some(start) = existing.find(start_marker) {
        if let Some(end_relative) = existing[start..].find(end_marker) {
            let end = start + end_relative + end_marker.len();
            format!(
                "{}{}{}",
                &existing[..start],
                managed_block,
                &existing[end..]
            )
        } else {
            format!("{}\n\n{}", existing.trim_end(), managed_block)
        }
    } else if existing.trim().is_empty() {
        format!("{heading}\n\n{managed_block}\n")
    } else {
        format!("{}\n\n{}\n", existing.trim_end(), managed_block)
    };

    fs::write(path, updated).map_err(|error| {
        format!(
            "failed to write bootstrap runtime self file {}: {error}",
            path.display()
        )
    })
}

fn persist_startup_personalization(
    runtime: &mut CliTurnRuntime,
    preset: StartupPersonalizationPreset,
    language: Language,
) -> CliResult<String> {
    let mut config = runtime.config.clone();
    let path = runtime.resolved_path.display().to_string();
    let now = OffsetDateTime::now_utc();
    let updated_at_epoch_seconds = u64::try_from(now.unix_timestamp()).ok();

    let message = if preset == StartupPersonalizationPreset::Later {
        config.memory.personalization = Some(PersonalizationConfig {
            prompt_state: PersonalizationPromptState::Deferred,
            updated_at_epoch_seconds,
            ..PersonalizationConfig::default()
        });
        match language {
            Language::ZhCn => "个性化暂时先不保存；Loong 会先保持首轮对话中性。".to_owned(),
            Language::ZhTw => "個性化暫時先不保存；Loong 會先保持首輪對話中性。".to_owned(),
            Language::En | Language::Ja | Language::Ru => {
                "personalization deferred; Loong will keep the first conversation neutral for now."
                    .to_owned()
            }
        }
    } else if preset == StartupPersonalizationPreset::TurnOff {
        config.memory.personalization = Some(PersonalizationConfig {
            prompt_state: PersonalizationPromptState::Suppressed,
            updated_at_epoch_seconds,
            ..PersonalizationConfig::default()
        });
        match language {
            Language::ZhCn => "已关闭后续个性化提示；Loong 会保持默认对话风格。".to_owned(),
            Language::ZhTw => "已關閉後續個性化提示；Loong 會保持預設對話風格。".to_owned(),
            Language::Ja => {
                "今後の個性化案内をオフにしました。Loong は標準の会話スタイルを保ちます。"
                    .to_owned()
            }
            Language::Ru => {
                "Дальнейшие подсказки по персонализации отключены; Loong сохранит стиль по умолчанию."
                    .to_owned()
            }
            Language::En => {
                "future personalization prompts turned off; Loong will keep the default conversation style."
                    .to_owned()
            }
        }
    } else {
        let mut upgraded_memory_profile = false;
        if config.memory.profile != MemoryProfile::ProfilePlusWindow {
            config.memory.profile = MemoryProfile::ProfilePlusWindow;
            upgraded_memory_profile = true;
        }
        config.memory.personalization = Some(PersonalizationConfig {
            response_density: preset.response_density(),
            initiative_level: preset.initiative_level(),
            locale: Some(startup_personalization_locale(language).to_owned()),
            prompt_state: PersonalizationPromptState::Configured,
            updated_at_epoch_seconds,
            ..PersonalizationConfig::default()
        });
        if upgraded_memory_profile {
            match language {
                Language::ZhCn => format!(
                    "已保存 {}，并把 memory.profile 升级为 profile_plus_window。",
                    preset.label(language)
                ),
                Language::ZhTw => format!(
                    "已保存 {}，並把 memory.profile 升級為 profile_plus_window。",
                    preset.label(language)
                ),
                Language::En | Language::Ja | Language::Ru => format!(
                    "saved {} and upgraded memory.profile to profile_plus_window.",
                    preset.label(language)
                ),
            }
        } else {
            match language {
                Language::ZhCn => {
                    format!("已把 {} 保存为首轮对话风格。", preset.label(language))
                }
                Language::ZhTw => {
                    format!("已把 {} 保存為首輪對話風格。", preset.label(language))
                }
                Language::En | Language::Ja | Language::Ru => format!(
                    "saved {} for the first real conversation.",
                    preset.label(language)
                ),
            }
        }
    };

    crate::config::write(Some(path.as_str()), &config, true)?;
    runtime.config = config;
    Ok(message)
}

