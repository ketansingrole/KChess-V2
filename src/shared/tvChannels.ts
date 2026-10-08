/** Lichess TV channels, keyed as the feed URLs name them. */
export const TV_CHANNELS: readonly { key: string; label: string }[] = [
  { key: 'best', label: 'Top rated' },
  { key: 'bullet', label: 'Bullet' },
  { key: 'blitz', label: 'Blitz' },
  { key: 'rapid', label: 'Rapid' },
  { key: 'classical', label: 'Classical' },
  { key: 'ultraBullet', label: 'UltraBullet' },
  { key: 'chess960', label: 'Chess960' },
  { key: 'kingOfTheHill', label: 'King of the Hill' },
  { key: 'threeCheck', label: 'Three-check' },
  { key: 'antichess', label: 'Antichess' },
  { key: 'atomic', label: 'Atomic' },
  { key: 'horde', label: 'Horde' },
  { key: 'racingKings', label: 'Racing Kings' },
  { key: 'crazyhouse', label: 'Crazyhouse' },
  { key: 'bot', label: 'Bots' },
  { key: 'computer', label: 'Computer' },
]
export const TV_CHANNEL_KEYS = TV_CHANNELS.map((channel) => channel.key)
