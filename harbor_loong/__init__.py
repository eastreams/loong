__all__ = ["LoongInstalledAgent"]


def __getattr__(name: str):
    if name != "LoongInstalledAgent":
        message = f"module {__name__!r} has no attribute {name!r}"
        raise AttributeError(message)

    from .agent import LoongInstalledAgent

    return LoongInstalledAgent
