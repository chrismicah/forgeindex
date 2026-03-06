"""Data models for the sample project."""

from typing import Optional


class User:
    """Represents a user in the system."""

    def __init__(self, name: str, email: str):
        """Initialize a User."""
        self.name = name
        self.email = email
        self._id: Optional[int] = None

    def display_name(self) -> str:
        """Get the user's display name."""
        return self.name

    def __repr__(self) -> str:
        return f"User(name={self.name!r}, email={self.email!r})"


class Product:
    """Represents a product in the catalog."""

    def __init__(self, name: str, price: float):
        """Initialize a Product."""
        self.name = name
        self.price = price

    def discounted_price(self, discount: float) -> float:
        """Calculate the discounted price."""
        return round(self.price * (1 - discount), 2)

    def __repr__(self) -> str:
        return f"Product(name={self.name!r}, price={self.price})"


class Order:
    """Represents a customer order."""

    def __init__(self, user: User):
        """Initialize an Order."""
        self.user = user
        self.items: list = []

    def add_item(self, product: Product, quantity: int = 1) -> None:
        """Add a product to the order."""
        self.items.append((product, quantity))

    def total(self) -> float:
        """Calculate order total."""
        return sum(p.price * q for p, q in self.items)
