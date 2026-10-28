import { clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

export const cn = (...values) => twMerge(clsx(values));
