import { BaseCalculator } from "./base";

export interface Numeric {
  value: number;
}

export function add(left: number, right: number): number {
  return left + right;
}

export class Calculator extends BaseCalculator {
  calculate(value: Numeric): number {
    return add(value.value, 2);
  }
}
