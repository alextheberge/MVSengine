from service_api import __all__ as SERVICE_EXPORTS
from service_api import *

__all__ = [*SERVICE_EXPORTS, "login"]


def login(username: str) -> str:
    return username
