#ifndef DEMO_FFI_H
#define DEMO_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct DemoCalculator DemoCalculator;

typedef int32_t DemoStatus;

#define DEMO_STATUS_OK             ((DemoStatus)0)
#define DEMO_STATUS_NULL_POINTER  ((DemoStatus)1)
#define DEMO_STATUS_INVALID_ARGUMENT ((DemoStatus)2)
#define DEMO_STATUS_INTERNAL_PANIC ((DemoStatus)3)

DemoCalculator* demo_calculator_create(double initial);

void demo_calculator_destroy(
    DemoCalculator* handle
);

DemoStatus demo_calculator_add(
    DemoCalculator* handle,
    double value
);

DemoStatus demo_calculator_multiply(
    DemoCalculator* handle,
    double value
);

DemoStatus demo_calculator_discount(
    DemoCalculator* handle,
    double percent
);

DemoStatus demo_calculator_value(
    const DemoCalculator* handle,
    double* out_value
);

const char* demo_status_string(
    DemoStatus status
);

#ifdef __cplusplus
}
#endif

#endif