/* Lichess API types KChess reads, copied from @lichess-org/types 2.0.176 (lichess-api.d.ts).
   Only the schemas the contracts and services reference are kept. Regenerate from that package
   when Lichess changes these shapes. */

export interface components {
  schemas: {
    Flair: string
    /**
     * @description only appears if the user is a titled player or a bot user
     * @enum {string}
     */
    Title: 'GM' | 'WGM' | 'IM' | 'WIM' | 'FM' | 'WFM' | 'NM' | 'CM' | 'WCM' | 'WNM' | 'LM' | 'BOT'
    /**
     * @deprecated
     * @description Use patronColor value instead to determine if player is a patron.
     */
    Patron: boolean
    /**
     * @description Players can choose a color for their Patron wings.
     *     See [here for the color mappings](https://github.com/lichess-org/lila/blob/master/ui/lib/css/abstract/_patron-colors.scss).
     *
     *     The presence of this field indicates the player is an active Patron.
     */
    PatronColor: number
    Perf: {
      games: number
      rating: number
      /** @description rating deviation */
      rd: number
      prog: number
      /** @description only appears if a user's perf rating are [provisional](https://lichess.org/faq#provisional) */
      prov?: boolean
      /** @description global lichess ranking, only appears for recently active players */
      rank?: number
    }
    PuzzleModePerf: {
      runs: number
      score: number
    }
    Perfs: {
      chess960?: components['schemas']['Perf']
      atomic?: components['schemas']['Perf']
      racingKings?: components['schemas']['Perf']
      ultraBullet?: components['schemas']['Perf']
      blitz?: components['schemas']['Perf']
      kingOfTheHill?: components['schemas']['Perf']
      threeCheck?: components['schemas']['Perf']
      antichess?: components['schemas']['Perf']
      crazyhouse?: components['schemas']['Perf']
      bullet?: components['schemas']['Perf']
      correspondence?: components['schemas']['Perf']
      horde?: components['schemas']['Perf']
      puzzle?: components['schemas']['Perf']
      classical?: components['schemas']['Perf']
      rapid?: components['schemas']['Perf']
      storm?: components['schemas']['PuzzleModePerf']
      racer?: components['schemas']['PuzzleModePerf']
      streak?: components['schemas']['PuzzleModePerf']
    }
    Profile: {
      flag?: string
      location?: string
      bio?: string
      realName?: string
      /** @description only appears if a user has set them */
      fideRating?: number
      /** @description only appears if a user has set them */
      uscfRating?: number
      /** @description only appears if a user has set them */
      ecfRating?: number
      /** @description only appears if a user has set them */
      cfcRating?: number
      /** @description only appears if a user has set them */
      rcfRating?: number
      /** @description only appears if a user has set them */
      dsbRating?: number
      links?: string
    }
    PlayTime: {
      total: number
      tv: number
      human?: number
    }
    User: {
      id: string
      username: string
      perfs?: components['schemas']['Perfs']
      title?: components['schemas']['Title']
      flair?: components['schemas']['Flair']
      /** Format: int64 */
      createdAt?: number
      /** @description only appears if a user's account is closed */
      disabled?: boolean
      /** @description only appears if a user's account is marked for the violation of [Lichess TOS](https://lichess.org/terms-of-service) */
      tosViolation?: boolean
      profile?: components['schemas']['Profile']
      /** Format: int64 */
      seenAt?: number
      playTime?: components['schemas']['PlayTime']
      patron?: components['schemas']['Patron']
      patronColor?: components['schemas']['PatronColor']
      verified?: boolean
    }
    Count: {
      all: number
      rated: number
      ai?: number
      draw: number
      drawH?: number
      loss: number
      lossH?: number
      win: number
      winH?: number
      bookmark: number
      playing: number
      import: number
      me: number
    }
    UserStreamer: {
      twitch?: {
        /** Format: uri */
        channel?: string
      }
      youtube?: {
        /** Format: uri */
        channel?: string
      }
    }
    UserExtended: components['schemas']['User'] & {
      /** Format: uri */
      url: string
      /** Format: uri */
      playing?: string
      count?: components['schemas']['Count']
      streaming?: boolean
      streamer?: components['schemas']['UserStreamer']
      /** @description only appears if the request is [authenticated with OAuth2](#description/authentication) */
      followable?: boolean
      /** @description only appears if the request is [authenticated with OAuth2](#description/authentication) */
      following?: boolean
      /** @description only appears if the request is [authenticated with OAuth2](#description/authentication) */
      blocking?: boolean
      fideId?: number
    }
    /** @enum {string} */
    PerfType:
      | 'ultraBullet'
      | 'bullet'
      | 'blitz'
      | 'rapid'
      | 'classical'
      | 'correspondence'
      | 'chess960'
      | 'crazyhouse'
      | 'antichess'
      | 'atomic'
      | 'horde'
      | 'kingOfTheHill'
      | 'racingKings'
      | 'threeCheck'
    RatingHistoryEntry: {
      name?: 'puzzle' | components['schemas']['PerfType']
      points?: number[][]
    }
    RatingHistory: components['schemas']['RatingHistoryEntry'][]
    GameColor: 'white' | 'black'
    /**
     * @default standard
     * @enum {string}
     */
    VariantKey:
      | 'standard'
      | 'chess960'
      | 'crazyhouse'
      | 'antichess'
      | 'atomic'
      | 'horde'
      | 'kingOfTheHill'
      | 'racingKings'
      | 'threeCheck'
      | 'fromPosition'
    Speed: 'ultraBullet' | 'bullet' | 'blitz' | 'rapid' | 'classical' | 'correspondence'
    /** @enum {string} */
    GameStatusName:
      | 'created'
      | 'started'
      | 'aborted'
      | 'mate'
      | 'resign'
      | 'stalemate'
      | 'timeout'
      | 'draw'
      | 'outoftime'
      | 'cheat'
      | 'noStart'
      | 'unknownFinish'
      | 'insufficientMaterialClaim'
      | 'variantEnd'
    GameStatusId: 10 | 20 | 25 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 60
    GameSource:
      | 'lobby'
      | 'friend'
      | 'ai'
      | 'api'
      | 'tournament'
      | 'position'
      | 'import'
      | 'importlive'
      | 'simul'
      | 'relay'
      | 'pool'
      | 'arena'
      | 'swiss'
    GameStatus: {
      id: components['schemas']['GameStatusId']
      name: components['schemas']['GameStatusName']
    }
    Variant: {
      key: components['schemas']['VariantKey']
      name: string
      short?: string
    }
    GameEventOpponent:
      | {
          id: string
          username: string
          rating: number
          ratingDiff?: number
        }
      | {
          id: null
          username: string
          /** @description AI level, from 1 to 8, where 1 is the weakest and 8 is the strongest. */
          ai: number
        }
    GameCompat: {
      /** @description Compatible with Bot API */
      bot?: boolean
      /** @description Compatible with Board API */
      board?: boolean
    }
    GameEventInfo: {
      fullId: string
      gameId: string
      fen?: string
      color?: components['schemas']['GameColor']
      lastMove?: string
      source?: components['schemas']['GameSource']
      status?: components['schemas']['GameStatus']
      variant?: components['schemas']['Variant']
      speed?: components['schemas']['Speed']
      perf?: string
      rating?: number
      rated?: boolean
      hasMoved?: boolean
      opponent?: components['schemas']['GameEventOpponent']
      isMyTurn?: boolean
      secondsLeft?: number
      winner?: components['schemas']['GameColor']
      ratingDiff?: number
      compat?: components['schemas']['GameCompat']
      id?: string
      tournamentId?: string
    }
    GameStartEvent: {
      /** @constant */
      type: 'gameStart'
      game: components['schemas']['GameEventInfo']
    }
    GameFinishEvent: {
      /** @constant */
      type: 'gameFinish'
      game: components['schemas']['GameEventInfo']
    }
    /** @enum {string} */
    ChallengeStatus: 'created' | 'offline' | 'canceled' | 'declined' | 'accepted'
    ChallengeUser: {
      id: string
      name: string
      rating?: number
      title?: components['schemas']['Title']
      flair?: components['schemas']['Flair']
      patron?: components['schemas']['Patron']
      patronColor?: components['schemas']['PatronColor']
      provisional?: boolean
      online?: boolean
      lag?: number
    }
    TimeControl:
      | {
          /** @constant */
          type: 'clock'
          limit?: number
          increment?: number
          show?: string
        }
      | {
          /** @constant */
          type: 'correspondence'
          daysPerTurn?: number
        }
      | {
          /** @constant */
          type: 'unlimited'
        }
    /** @enum {string} */
    ChallengeColor: 'white' | 'black' | 'random'
    ChallengeJson: {
      id: string
      /** Format: uri */
      url: string
      status: components['schemas']['ChallengeStatus']
      challenger: components['schemas']['ChallengeUser']
      destUser: components['schemas']['ChallengeUser'] | null
      variant: components['schemas']['Variant']
      rated: boolean
      speed: components['schemas']['Speed']
      timeControl: components['schemas']['TimeControl']
      color: components['schemas']['ChallengeColor']
      finalColor?: components['schemas']['GameColor']
      perf: {
        icon: string
        name: string
      }
      /** @enum {string} */
      direction?: 'in' | 'out'
      initialFen?: string
      rematchOf?: string
    }
    ChallengeEvent: {
      /** @constant */
      type: 'challenge'
      challenge: components['schemas']['ChallengeJson']
      compat?: components['schemas']['GameCompat']
    }
    ChallengeCanceledEvent: {
      /** @constant */
      type: 'challengeCanceled'
      challenge: components['schemas']['ChallengeJson']
    }
    ChallengeDeclinedJson: components['schemas']['ChallengeJson'] & {
      /** @description Human readable, possibly translated reason why the challenge was declined. */
      declineReason: string
      /**
       * @description Untranslated, computer-matchable reason why the challenge was declined.
       * @enum {string}
       */
      declineReasonKey:
        | 'generic'
        | 'later'
        | 'toofast'
        | 'tooslow'
        | 'timecontrol'
        | 'rated'
        | 'casual'
        | 'standard'
        | 'variant'
        | 'nobot'
        | 'onlybot'
    }
    ChallengeDeclinedEvent: {
      /** @constant */
      type: 'challengeDeclined'
      challenge: components['schemas']['ChallengeDeclinedJson']
    }
    GameEventPlayer: {
      aiLevel?: number
      id: string
      name: string
      title?: components['schemas']['Title'] | null
      rating?: number
      ratingDiff?: number
      provisional?: boolean
    }
    GameStateEvent: {
      /** @constant */
      type: 'gameState'
      /**
       * @description Current moves in UCI format (King to rook for Chess690-compatible castling
       *     notation)
       */
      moves: string
      /** @description Integer of milliseconds White has left on the clock */
      wtime: number
      /** @description Integer of milliseconds Black has left on the clock */
      btime: number
      /** @description Integer of White Fisher increment. */
      winc: number
      /** @description Integer of Black Fisher increment. */
      binc: number
      status: components['schemas']['GameStatusName']
      /** @description Color of the winner, if any */
      winner?: components['schemas']['GameColor']
      /** @description true if white is offering draw, else omitted */
      wdraw?: boolean
      /** @description true if black is offering draw, else omitted */
      bdraw?: boolean
      /** @description true if white is proposing takeback, else omitted */
      wtakeback?: boolean
      /** @description true if black is proposing takeback, else omitted */
      btakeback?: boolean
      /** @description A game may be aborted if a player doesn't make their first move in time */
      expiration?: {
        /** @description Milliseconds since the last move was played, or since the game started */
        idleMillis: number
        /** @description Time each player has to make their first move, before the game is aborted */
        millisToMove: number
      }
    }
    GameFullEvent: {
      /** @constant */
      type: 'gameFull'
      id: string
      variant: components['schemas']['Variant']
      clock?: {
        /**
         * Format: int64
         * @description Initial time in milliseconds
         */
        initial?: number
        /**
         * Format: int64
         * @description Increment time in milliseconds
         */
        increment?: number
      }
      speed: components['schemas']['Speed']
      perf: {
        /** @description Translated perf name (e.g. "Classical" or "Blitz") */
        name?: string
      }
      rated: boolean
      /** Format: int64 */
      createdAt: number
      white: components['schemas']['GameEventPlayer']
      black: components['schemas']['GameEventPlayer']
      /** @default startpos */
      initialFen: string
      state: components['schemas']['GameStateEvent']
      /** @description If the game is correspondence */
      daysPerTurn?: number
      tournamentId?: string
    }
    ChatLineEvent: {
      /** @constant */
      type: 'chatLine'
      /** @enum {string} */
      room: 'player' | 'spectator'
      username: string
      text: string
    }
    OpponentGoneEvent: {
      /** @constant */
      type: 'opponentGone'
      gone: boolean
      claimWinInSeconds?: number
    }
  }
}

export interface paths {
  '/api/account/playing': {
    get: {
      responses: {
        200: {
          content: {
            'application/json': {
              /** @description Number of games where it is my turn to play */
              nbMyTurn: number
              /** @description Games I'm currently playing */
              nowPlaying: {
                fullId: string
                gameId: string
                fen: string
                color: components['schemas']['GameColor']
                lastMove: string
                source: components['schemas']['GameSource']
                status?: components['schemas']['GameStatus']
                variant: components['schemas']['Variant']
                speed: components['schemas']['Speed']
                perf: components['schemas']['PerfType']
                rated: boolean
                rating: number
                hasMoved: boolean
                opponent:
                  | {
                      id: string
                      username: string
                      rating?: number
                      ratingDiff?: number
                    }
                  | {
                      id: null
                      username: string
                    }
                  | {
                      id: null
                      username: string
                      /** @description AI level, from 1 to 8. */
                      ai: number
                    }
                isMyTurn: boolean
                secondsLeft: number
                tournamentId?: string
                swissId?: string
                winner?: components['schemas']['GameColor']
                ratingDiff?: number
              }[]
            }
          }
        }
      }
    }
  }
}
