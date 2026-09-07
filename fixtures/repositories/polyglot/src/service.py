"""Billing helpers used by the polyglot fixture."""

import json
from decimal import Decimal

TAX_RATE = Decimal("0.2")


def apply_tax(amount):
    """Applies the standard tax rate to an amount."""
    return amount * (1 + TAX_RATE)


class InvoiceBuilder:
    """Builds invoices from line items."""

    def __init__(self, customer):
        self.customer = customer
        self.items = []

    def add_item(self, description, amount):
        # Records one billable line.
        self.items.append({"description": description, "amount": apply_tax(amount)})
        return self

    def render(self):
        """Serialises the invoice as JSON."""
        return json.dumps({"customer": self.customer, "items": self.items})
