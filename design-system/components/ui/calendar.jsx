import * as React from 'react';
import { DayPicker, getDefaultClassNames } from 'react-day-picker';
import { ko } from 'react-day-picker/locale';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { cn } from './utils.js';

export function Calendar({ className, classNames, components, labels, formatters, ...props }) {
  const defaults = getDefaultClassNames();
  return <DayPicker
    showOutsideDays fixedWeeks locale={ko} weekStartsOn={0} captionLayout="label"
    className={cn('grove-day-picker', className)}
    classNames={{ ...defaults, ...classNames }}
    labels={{ labelPrevious: () => '이전 달', labelNext: () => '다음 달', labelNav: () => '월 이동', ...labels }}
    formatters={{ formatCaption: date => `${date.getFullYear()}년 ${date.getMonth() + 1}월`, formatWeekdayName: date => ['일', '월', '화', '수', '목', '금', '토'][date.getDay()], ...formatters }}
    components={{ Chevron: ({ orientation, ...iconProps }) => orientation === 'left' ? <ChevronLeft size={16} {...iconProps} /> : <ChevronRight size={16} {...iconProps} />, ...components }}
    {...props}
  />;
}
