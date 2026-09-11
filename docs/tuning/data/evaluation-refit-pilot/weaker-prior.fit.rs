// Fitted by `tune fit`. Paste into the files named below.

// ---- src/engine/evaluation/weights.rs ----
const PAWN: ScorePair = ScorePair::new(94, 149);
const KNIGHT: ScorePair = ScorePair::new(350, 308);
const BISHOP: ScorePair = ScorePair::new(371, 353);
const ROOK: ScorePair = ScorePair::new(537, 589);
const QUEEN: ScorePair = ScorePair::new(1019, 1022);
const ACTIVITY: ScorePair = ScorePair::new(2, -6);
const TEMPO: ScorePair = ScorePair::new(23, 4);
const PAWN_KING_MOBILITY: ScorePair = ScorePair::new(-3, -2);
const BISHOP_PAIR: ScorePair = ScorePair::new(37, 56);
const DOUBLED_PAWN: ScorePair = ScorePair::new(-9, -18);
const ISOLATED_PAWN: ScorePair = ScorePair::new(-2, -10);
const PASSED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-3, 6),
    ScorePair::new(1, 28),
    ScorePair::new(8, 64),
    ScorePair::new(25, 96),
    ScorePair::new(41, 103),
    ScorePair::new(33, 89),
];
const PROTECTED_PASSED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(0, 0),
    ScorePair::new(1, -7),
    ScorePair::new(1, 1),
    ScorePair::new(11, 1),
    ScorePair::new(9, 7),
    ScorePair::new(0, -3),
];
const KING_SHELTER: ScorePair = ScorePair::new(27, -12);
const OPEN_KING_FILE: ScorePair = ScorePair::new(-17, -3);

// ---- src/engine/evaluation/placement.rs ----
static PAWN: Table = Table {
    middle_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        99, 142, 63, 95, 62, 134, 22, -31, //
        -28, -11, 16, 27, 51, 42, 14, -32, //
        -26, -2, -15, 12, 8, 3, -1, -37, //
        -44, -22, -18, -2, 2, -3, -21, -48, //
        -50, -32, -26, -32, -18, -24, -16, -40, //
        -50, -18, -28, -36, -33, -5, 8, -53, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
    end_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        153, 150, 144, 110, 117, 106, 146, 172, //
        59, 68, 40, 19, 6, 1, 54, 52, //
        -1, -15, -23, -44, -43, -45, -29, -24, //
        -19, -38, -51, -57, -65, -50, -41, -46, //
        -37, -44, -62, -59, -69, -53, -63, -60, //
        -28, -46, -54, -30, -43, -57, -56, -43, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
};
static KNIGHT: Table = Table {
    middle_game: [
        -202, -108, -40, -58, 75, -117, -17, -127, //
        -86, -50, 84, 44, 28, 74, 8, -21, //
        -55, 71, 46, 79, 103, 155, 88, 52, //
        -7, 23, 29, 71, 39, 80, 25, 38, //
        -18, 5, 23, 18, 30, 36, 29, -4, //
        -38, 1, 11, 21, 21, 22, 23, -22, //
        -33, -61, -22, -2, -5, 17, -17, -21, //
        -127, -25, -68, -27, -15, -30, -20, -27, //
    ],
    end_game: [
        -48, -26, 3, -12, -15, -14, -57, -100, //
        -12, 12, -15, 14, 11, -12, -11, -42, //
        -7, -5, 37, 32, 18, 10, 1, -32, //
        -1, 22, 45, 43, 47, 27, 27, 2, //
        -6, 14, 37, 48, 33, 42, 26, 3, //
        -14, 19, 16, 30, 24, 7, -8, -5, //
        -30, -4, 5, 11, 20, -11, -10, -35, //
        -16, -38, -6, 6, -2, -2, -37, -58, //
    ],
};
static BISHOP: Table = Table {
    middle_game: [
        -42, -2, -108, -50, -37, -58, 2, -16, //
        -39, 3, -26, -21, 29, 63, 13, -63, //
        -24, 37, 39, 40, 37, 56, 42, -5, //
        -12, 0, 15, 45, 28, 36, 2, -6, //
        -13, 10, 13, 29, 30, 2, 7, -3, //
        6, 15, 16, 6, 9, 21, 9, 6, //
        -4, 6, 14, -1, 7, 17, 37, -5, //
        -44, -5, -6, -33, -20, -22, -54, -29, //
    ],
    end_game: [
        -11, -16, -6, -1, 0, -5, -16, -24, //
        -3, -8, 15, -7, 4, -9, 0, -13, //
        10, -4, 5, 7, 1, 14, 7, 8, //
        -1, 13, 18, 10, 16, 13, 8, 8, //
        -1, 3, 20, 24, 10, 13, 8, -9, //
        -9, 2, 18, 12, 18, 5, -7, -14, //
        -9, -13, -7, 8, 6, -5, -20, -27, //
        -17, -5, -13, 1, -4, -9, -1, -12, //
    ],
};
static ROOK: Table = Table {
    middle_game: [
        28, 41, 28, 53, 65, 2, 30, 38, //
        23, 31, 52, 68, 90, 74, 22, 42, //
        -11, 16, 26, 35, 16, 47, 64, 11, //
        -34, -20, 2, 27, 14, 30, -16, -33, //
        -49, -43, -24, -19, -2, -17, -3, -38, //
        -62, -44, -26, -33, -10, -7, -10, -53, //
        -60, -27, -35, -18, -5, 8, -17, -100, //
        -35, -19, -24, 0, 7, -10, -39, -43, //
    ],
    end_game: [
        14, 5, 16, 13, 7, 13, 11, -6, //
        10, 12, 4, 10, -1, 7, 11, -2, //
        13, 9, 13, 6, 6, -4, -5, -3, //
        15, 11, 20, 2, 0, 2, 3, 4, //
        11, 7, 14, 5, -7, -1, -8, -15, //
        -4, -4, -5, -3, -9, -12, -8, -17, //
        -10, -7, -3, -2, -13, -16, -19, -8, //
        -8, -6, -1, -15, -20, -18, 1, -17, //
    ],
};
static QUEEN: Table = Table {
    middle_game: [
        -35, -1, 33, 8, 67, 50, 47, 46, //
        -24, -46, -8, -2, -22, 65, 31, 55, //
        -13, -26, 8, 9, 33, 61, 52, 53, //
        -37, -31, -18, -26, -5, 20, -10, -4, //
        -8, -34, -8, -13, -8, -10, -4, -6, //
        -11, 2, -15, -7, -3, 1, 14, 2, //
        -44, -11, 6, 2, 6, 18, -10, -4, //
        -10, -25, 2, 12, -26, -31, -41, -66, //
    ],
    end_game: [
        -21, 16, 16, 20, 21, 10, 1, 11, //
        -29, 15, 28, 39, 59, 19, 25, -12, //
        -32, -4, 1, 47, 46, 30, 14, -1, //
        -8, 16, 19, 43, 58, 39, 57, 33, //
        -30, 23, 16, 45, 26, 32, 34, 18, //
        -28, -39, 7, -7, 4, 12, 2, -7, //
        -38, -39, -49, -28, -28, -39, -57, -51, //
        -52, -44, -34, -58, -22, -51, -36, -60, //
    ],
};
static KING: Table = Table {
    middle_game: [
        -60, 46, 38, 1, -50, -22, 21, 35, //
        54, 17, -5, 11, 10, 14, -27, -16, //
        7, 49, 21, -1, -5, 25, 44, -7, //
        -2, -5, 4, -14, -19, -12, 1, -25, //
        -40, 15, -15, -30, -39, -36, -23, -45, //
        1, 2, -12, -37, -36, -21, -2, -14, //
        21, 31, 7, -43, -26, 10, 25, 29, //
        4, 50, 41, -33, 9, -10, 49, 36, //
    ],
    end_game: [
        -93, -45, -25, -25, -16, 15, 1, -24, //
        -18, 17, 18, 17, 16, 42, 25, 8, //
        8, 20, 28, 17, 27, 44, 46, 12, //
        -15, 24, 28, 27, 30, 35, 28, 4, //
        -26, -7, 21, 27, 22, 21, 8, -18, //
        -27, -1, 4, 23, 28, 9, -2, -20, //
        -36, -16, 0, 21, 17, 5, -11, -39, //
        -61, -48, -21, -6, -34, -6, -40, -66, //
    ],
};

// ---- src/engine/evaluation/weights.rs ----
const KNIGHT_MOBILITY: [ScorePair; 9] = [
    ScorePair::new(-3, 0),
    ScorePair::new(4, 10),
    ScorePair::new(10, 25),
    ScorePair::new(18, 29),
    ScorePair::new(16, 40),
    ScorePair::new(17, 42),
    ScorePair::new(20, 48),
    ScorePair::new(24, 51),
    ScorePair::new(20, 51),
];
const BISHOP_MOBILITY: [ScorePair; 14] = [
    ScorePair::new(-15, -4),
    ScorePair::new(-2, 9),
    ScorePair::new(9, 20),
    ScorePair::new(10, 31),
    ScorePair::new(14, 39),
    ScorePair::new(26, 53),
    ScorePair::new(29, 57),
    ScorePair::new(35, 63),
    ScorePair::new(28, 69),
    ScorePair::new(29, 76),
    ScorePair::new(31, 78),
    ScorePair::new(34, 78),
    ScorePair::new(43, 99),
    ScorePair::new(46, 99),
];
const ROOK_MOBILITY: [ScorePair; 15] = [
    ScorePair::new(-18, -4),
    ScorePair::new(0, 12),
    ScorePair::new(6, 26),
    ScorePair::new(16, 36),
    ScorePair::new(22, 40),
    ScorePair::new(24, 49),
    ScorePair::new(21, 65),
    ScorePair::new(28, 68),
    ScorePair::new(27, 75),
    ScorePair::new(32, 80),
    ScorePair::new(32, 84),
    ScorePair::new(32, 86),
    ScorePair::new(38, 92),
    ScorePair::new(43, 99),
    ScorePair::new(43, 80),
];
const QUEEN_MOBILITY: [ScorePair; 28] = [
    ScorePair::new(0, 0),
    ScorePair::new(6, 7),
    ScorePair::new(9, 17),
    ScorePair::new(17, 25),
    ScorePair::new(18, 35),
    ScorePair::new(23, 44),
    ScorePair::new(29, 52),
    ScorePair::new(33, 65),
    ScorePair::new(36, 72),
    ScorePair::new(38, 81),
    ScorePair::new(40, 89),
    ScorePair::new(42, 100),
    ScorePair::new(48, 106),
    ScorePair::new(53, 118),
    ScorePair::new(58, 125),
    ScorePair::new(54, 131),
    ScorePair::new(61, 141),
    ScorePair::new(59, 149),
    ScorePair::new(61, 154),
    ScorePair::new(67, 161),
    ScorePair::new(71, 168),
    ScorePair::new(74, 175),
    ScorePair::new(79, 184),
    ScorePair::new(83, 192),
    ScorePair::new(86, 202),
    ScorePair::new(91, 212),
    ScorePair::new(95, 221),
    ScorePair::new(98, 230),
];
const ROOK_OPEN_FILE: ScorePair = ScorePair::new(39, -10);
const ROOK_SEMI_OPEN_FILE: ScorePair = ScorePair::new(6, 21);
const ROOK_ON_SEVENTH: ScorePair = ScorePair::new(-3, 10);
const KNIGHT_OUTPOST: ScorePair = ScorePair::new(22, 9);
const BISHOP_OUTPOST: ScorePair = ScorePair::new(6, 14);
const BACKWARD_PAWN: ScorePair = ScorePair::new(1, -6);
const THREAT_MINOR_BY_PAWN: ScorePair = ScorePair::new(50, 23);
const THREAT_HANGING: ScorePair = ScorePair::new(25, 22);
const THREAT_BY_LOWER_VALUE: ScorePair = ScorePair::new(38, 13);
const CONNECTED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(3, -7),
    ScorePair::new(14, 10),
    ScorePair::new(11, 13),
    ScorePair::new(23, 20),
    ScorePair::new(13, 16),
    ScorePair::new(0, -1),
];
const BLOCKED_PASSER_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-4, -11),
    ScorePair::new(-4, -3),
    ScorePair::new(-11, -7),
    ScorePair::new(-4, -17),
    ScorePair::new(-4, -21),
    ScorePair::new(-10, -32),
];
const PASSER_OWN_KING_DISTANCE: [ScorePair; 8] = [
    ScorePair::new(0, 7),
    ScorePair::new(1, 30),
    ScorePair::new(-3, 10),
    ScorePair::new(-2, -4),
    ScorePair::new(-10, -15),
    ScorePair::new(-6, -20),
    ScorePair::new(3, -16),
    ScorePair::new(-7, -13),
];
const PASSER_ENEMY_KING_DISTANCE: [ScorePair; 8] = [
    ScorePair::new(-16, -39),
    ScorePair::new(-5, -44),
    ScorePair::new(-2, -20),
    ScorePair::new(3, 2),
    ScorePair::new(-4, 21),
    ScorePair::new(5, 27),
    ScorePair::new(-2, 19),
    ScorePair::new(-1, 12),
];
const KING_DANGER_BY_BUCKET: [ScorePair; 16] = [
    ScorePair::new(0, -2),
    ScorePair::new(-5, -13),
    ScorePair::new(-12, -6),
    ScorePair::new(-3, -12),
    ScorePair::new(-2, -14),
    ScorePair::new(1, -14),
    ScorePair::new(6, 1),
    ScorePair::new(8, 3),
    ScorePair::new(4, 2),
    ScorePair::new(2, 2),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
    ScorePair::new(0, 0),
];
const SAFE_CHECK_BY_PIECE: [ScorePair; 4] = [
    ScorePair::new(24, -1),
    ScorePair::new(16, 16),
    ScorePair::new(26, 8),
    ScorePair::new(47, 15),
];
const SHELTER_KING_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(14, -6),
    ScorePair::new(4, 1),
    ScorePair::new(-2, -9),
    ScorePair::new(1, -2),
    ScorePair::new(0, -2),
    ScorePair::new(0, 0),
];
const SHELTER_ADJACENT_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(-5, -2),
    ScorePair::new(-10, 4),
    ScorePair::new(2, -4),
    ScorePair::new(-2, -6),
    ScorePair::new(0, -3),
    ScorePair::new(0, 0),
];
const STORM_KING_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(5, 11),
    ScorePair::new(0, 1),
    ScorePair::new(4, 2),
    ScorePair::new(3, 6),
    ScorePair::new(6, 2),
    ScorePair::new(6, 2),
];
const STORM_ADJACENT_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(0, 0),
    ScorePair::new(-3, -2),
    ScorePair::new(-4, 4),
    ScorePair::new(-2, 3),
    ScorePair::new(7, 3),
    ScorePair::new(4, -1),
];
const BLOCKED_STORM_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(0, 0),
    ScorePair::new(-3, -4),
    ScorePair::new(1, -1),
    ScorePair::new(2, -2),
    ScorePair::new(3, -1),
    ScorePair::new(-1, -2),
];
const SPACE_AREA: ScorePair = ScorePair::new(-8, 2);
const SPACE_AREA_BY_PIECES: ScorePair = ScorePair::new(1, -1);
const CANDIDATE_PASSER_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(2, -1),
    ScorePair::new(3, 2),
    ScorePair::new(7, 11),
    ScorePair::new(3, 7),
    ScorePair::new(1, 1),
    ScorePair::new(0, 0),
];
const PAWN_ISLANDS: ScorePair = ScorePair::new(-4, -1);
const KING_PAWNLESS_FLANK: ScorePair = ScorePair::new(-3, -19);
const KNIGHT_PER_PAWN: ScorePair = ScorePair::new(4, 6);
const BISHOP_PER_PAWN: ScorePair = ScorePair::new(5, 3);
const ROOK_PER_PAWN: ScorePair = ScorePair::new(-8, 5);
const BISHOP_PAWNS_ON_COLOUR: ScorePair = ScorePair::new(-6, -5);
const UNSAFE_MOBILITY_BY_PIECE: [ScorePair; 4] = [
    ScorePair::new(-9, -3),
    ScorePair::new(-1, -5),
    ScorePair::new(-5, 1),
    ScorePair::new(-5, 13),
];
const PASSER_SAFE_PATH_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-1, 2),
    ScorePair::new(-2, 0),
    ScorePair::new(-2, 2),
    ScorePair::new(0, 10),
    ScorePair::new(2, 13),
    ScorePair::new(1, 3),
];
const PASSER_FREE_PATH_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-1, 1),
    ScorePair::new(-2, 0),
    ScorePair::new(0, 1),
    ScorePair::new(1, 8),
    ScorePair::new(3, 7),
    ScorePair::new(2, 1),
];
const ROOK_BEHIND_PASSER: ScorePair = ScorePair::new(6, 4);
const THREAT_BY_PAWN_PUSH: ScorePair = ScorePair::new(11, 7);
const CASTLING_RIGHTS: ScorePair = ScorePair::new(21, 4);
const TROPISM_BY_PIECE_DISTANCE: [ScorePair; 16] = [
    ScorePair::new(-1, -2),
    ScorePair::new(-2, -4),
    ScorePair::new(2, 1),
    ScorePair::new(-7, 0),
    ScorePair::new(-1, -1),
    ScorePair::new(3, -3),
    ScorePair::new(0, -1),
    ScorePair::new(-6, 3),
    ScorePair::new(0, 0),
    ScorePair::new(0, -3),
    ScorePair::new(-1, 0),
    ScorePair::new(-8, 0),
    ScorePair::new(0, 0),
    ScorePair::new(8, 5),
    ScorePair::new(5, 3),
    ScorePair::new(3, 4),
];
