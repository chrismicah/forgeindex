"""Main application module for the sample project."""

from utils import calculate_total, format_currency
from models import User, Product

MAX_RETRIES = 3
DEFAULT_TIMEOUT = 30


class Application:
    """Main application class."""

    def __init__(self, name: str, debug: bool = False):
        """Initialize the application."""
        self.name = name
        self.debug = debug
        self._users = []
        self._products = []

    def run(self) -> None:
        """Start the application."""
        if self.debug:
            print(f"Starting {self.name} in debug mode")
        self._load_data()
        self._process()

    def _load_data(self) -> None:
        """Load initial data."""
        self._users = [
            User("Alice", "alice@example.com"),
            User("Bob", "bob@example.com"),
        ]
        self._products = [
            Product("Widget", 9.99),
            Product("Gadget", 19.99),
        ]

    def _process(self) -> None:
        """Process all pending operations."""
        for user in self._users:
            total = calculate_total([p.price for p in self._products])
            print(f"{user.name}: {format_currency(total)}")

    def add_user(self, user: User) -> None:
        """Add a new user to the application."""
        self._users.append(user)


def create_app(name: str = "MyApp") -> Application:
    """Factory function to create a new Application instance."""
    return Application(name)


if __name__ == "__main__":
    app = create_app()
    app.run()
