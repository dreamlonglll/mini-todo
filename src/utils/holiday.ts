/**
 * 节假日服务模块
 * 通过 Tauri 后端调用 NateScarlet/holiday-cn 开源数据获取中国法定节假日
 */
import { invoke } from '@tauri-apps/api/core'

export interface HolidayInfo {
  /** 日期 YYYY-MM-DD */
  date: string
  /** 是否是休息日 */
  isHoliday: boolean
  /** 是否是工作日（包括调休） */
  isWorkday: boolean
  /** 是否是调休日（周末需要上班） */
  isAdjust: boolean
  /** 节日名称（如：春节、国庆节），非节日则为空 */
  name: string
  /** 节日类型：1-法定节假日 2-调休上班 */
  type: number
}

// 后端返回的节假日数据格式
interface BackendHolidayInfo {
  date: string
  name: string
  isOffDay: boolean
}

// 年度节假日缓存
const holidayCache = new Map<number, Map<string, HolidayInfo>>()

/**
 * 从 Tauri 后端获取指定年份的节假日数据
 * @param year 年份
 */
async function fetchYearHolidays(year: number): Promise<Map<string, HolidayInfo>> {
  try {
    const holidays = await invoke<BackendHolidayInfo[]>('fetch_holidays', { year })
    const result = new Map<string, HolidayInfo>()
    
    for (const info of holidays) {
      result.set(info.date, {
        date: info.date,
        isHoliday: info.isOffDay,
        isWorkday: !info.isOffDay,
        isAdjust: !info.isOffDay, // 如果不是休息日但在节假日数据中，说明是调休上班
        name: info.name,
        type: info.isOffDay ? 1 : 2
      })
    }
    
    return result
  } catch (error) {
    console.error(`Failed to fetch holidays for ${year}:`, error)
    return new Map()
  }
}

/**
 * 获取指定年份的节假日数据（带缓存）
 * @param year 年份
 */
export async function getYearHolidays(year: number): Promise<Map<string, HolidayInfo>> {
  // 检查缓存
  if (holidayCache.has(year)) {
    return holidayCache.get(year)!
  }
  
  // 从 API 获取
  const holidays = await fetchYearHolidays(year)
  
  // 存入缓存
  if (holidays.size > 0) {
    holidayCache.set(year, holidays)
  }
  
  return holidays
}

