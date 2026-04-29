use super::*;

pub(super) fn append_turn(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::append_turn(request, config)
    }
}

pub(super) fn load_window(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::load_window(request, config)
    }
}

pub(super) fn load_transcript(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::load_transcript(request, config)
    }
}

pub(super) fn clear_session(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::clear_session(request, config)
    }
}

pub(super) fn replace_turns(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::replace_turns(request, config)
    }
}
