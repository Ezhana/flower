#include <cstdlib>
#include <iomanip>
#include <iostream>

#include "flower.h"

static void require_ok(
    DemoStatus status,
    const char* operation
) {
    if (status != DEMO_STATUS_OK) {
        std::cerr
            << operation
            << " failed: "
            << demo_status_string(status)
            << '\n';

        std::exit(1);
    }
}

int main() {
    DemoCalculator* calc =
        demo_calculator_create(100.0);

    if (!calc) {
        std::cerr << "create failed\n";
        return 1;
    }

    require_ok(
        demo_calculator_add(calc, 50.0),
        "add"
    );

    require_ok(
        demo_calculator_multiply(calc, 2.0),
        "multiply"
    );

    require_ok(
        demo_calculator_discount(calc, 10.0),
        "discount"
    );

    double value = 0.0;

    require_ok(
        demo_calculator_value(calc, &value),
        "value"
    );

    std::cout
        << std::fixed
        << std::setprecision(2)
        << "value = "
        << value
        << '\n';

    demo_calculator_destroy(calc);

    return 0;
}