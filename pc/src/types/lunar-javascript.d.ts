declare module 'lunar-javascript' {
  export class Solar {
    static fromYmd(year: number, month: number, day: number): Solar
    static fromDate(date: Date): Solar
    getLunar(): Lunar
  }

  export class Lunar {
    getDayInChinese(): string
    getMonthInChinese(): string
    getYearInGanZhi(): string
    getDay(): number
    getJieQi(): string | null
    getFestivals(): string[]
    toString(): string
  }
}
