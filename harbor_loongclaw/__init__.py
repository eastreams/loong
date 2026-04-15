__all__ = ["LoongClawInstalledAgent"]


def __getattr__(name: str):
    if name != "LoongClawInstalledAgent":
        message = f"module {__name__!r} has no attribute {name!r}"
        raise AttributeError(message)

    from .agent import LoongClawInstalledAgent

    return LoongClawInstalledAgent
