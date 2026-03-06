"""Utility functions for the sample project."""

from typing import List

PI = 3.14159265359
CURRENCY_SYMBOL = "$"


def calculate_total(prices: List[float], tax_rate: float = 0.08) -> float:
    """Calculate the total price including tax."""
    subtotal = sum(prices)
    tax = subtotal * tax_rate
    return round(subtotal + tax, 2)


def format_currency(amount: float) -> str:
    """Format a number as currency string."""
    return f"{CURRENCY_SYMBOL}{amount:.2f}"


def clamp(value: float, min_val: float, max_val: float) -> float:
    """Clamp a value between min and max."""
    return max(min_val, min(value, max_val))


def _internal_helper(data: str) -> str:
    """Private helper function."""
    return data.strip().lower()
