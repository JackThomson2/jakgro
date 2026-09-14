// Fitted by `tune fit`. Paste into the files named below.

// ---- src/engine/evaluation/weights.rs ----
const PAWN: ScorePair = ScorePair::new(94, 151);
const KNIGHT: ScorePair = ScorePair::new(341, 299);
const BISHOP: ScorePair = ScorePair::new(363, 345);
const ROOK: ScorePair = ScorePair::new(524, 575);
const QUEEN: ScorePair = ScorePair::new(993, 996);
const ACTIVITY: ScorePair = ScorePair::new(3, -6);
const TEMPO: ScorePair = ScorePair::new(22, 3);
const PAWN_KING_MOBILITY: ScorePair = ScorePair::new(-1, -4);
const BISHOP_PAIR: ScorePair = ScorePair::new(37, 55);
const DOUBLED_PAWN: ScorePair = ScorePair::new(-10, -18);
const ISOLATED_PAWN: ScorePair = ScorePair::new(-3, -7);
const PASSED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-2, 8),
    ScorePair::new(0, 28),
    ScorePair::new(8, 64),
    ScorePair::new(25, 93),
    ScorePair::new(41, 101),
    ScorePair::new(33, 88),
];
const PROTECTED_PASSED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(0, 0),
    ScorePair::new(1, -6),
    ScorePair::new(1, 1),
    ScorePair::new(10, 1),
    ScorePair::new(8, 6),
    ScorePair::new(0, -3),
];
const KING_SHELTER: ScorePair = ScorePair::new(25, -14);
const OPEN_KING_FILE: ScorePair = ScorePair::new(-16, -5);

// ---- src/engine/evaluation/placement.rs ----
static PAWN: Table = Table {
    middle_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        97, 139, 61, 93, 60, 131, 21, -30, //
        -27, -10, 16, 27, 50, 41, 14, -31, //
        -25, -3, -15, 13, 8, 2, -2, -36, //
        -43, -23, -17, -2, 2, -5, -19, -45, //
        -53, -31, -24, -32, -17, -24, -14, -40, //
        -48, -16, -30, -36, -30, -4, 7, -51, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
    end_game: [
        0, 0, 0, 0, 0, 0, 0, 0, //
        149, 146, 140, 107, 114, 103, 142, 168, //
        57, 67, 39, 20, 6, 1, 53, 50, //
        -1, -15, -23, -42, -42, -45, -30, -24, //
        -19, -38, -50, -55, -64, -49, -40, -42, //
        -36, -44, -61, -57, -68, -53, -63, -59, //
        -26, -43, -53, -29, -40, -54, -53, -42, //
        0, 0, 0, 0, 0, 0, 0, 0, //
    ],
};
static KNIGHT: Table = Table {
    middle_game: [
        -197, -105, -39, -57, 73, -114, -17, -124, //
        -84, -49, 82, 43, 27, 72, 8, -21, //
        -54, 69, 45, 78, 100, 151, 86, 51, //
        -7, 23, 28, 70, 37, 78, 25, 37, //
        -18, 5, 23, 16, 29, 36, 28, -4, //
        -37, 0, 10, 20, 21, 20, 22, -21, //
        -32, -60, -22, 0, -4, 17, -17, -21, //
        -124, -24, -66, -26, -15, -29, -19, -26, //
    ],
    end_game: [
        -47, -25, 3, -12, -15, -14, -56, -98, //
        -12, 12, -15, 14, 11, -12, -11, -41, //
        -7, -5, 36, 31, 18, 10, 1, -31, //
        -1, 22, 43, 42, 45, 26, 26, 2, //
        -6, 14, 37, 46, 32, 41, 25, 3, //
        -14, 19, 16, 29, 24, 7, -8, -5, //
        -29, -4, 5, 10, 20, -11, -10, -34, //
        -16, -37, -6, 6, -2, -2, -36, -57, //
    ],
};
static BISHOP: Table = Table {
    middle_game: [
        -41, -2, -105, -49, -36, -57, 2, -16, //
        -38, 3, -25, -21, 28, 61, 13, -62, //
        -23, 36, 38, 39, 36, 55, 41, -5, //
        -12, -1, 15, 44, 28, 35, 1, -6, //
        -13, 10, 12, 28, 30, 2, 7, -3, //
        6, 16, 15, 5, 8, 21, 8, 6, //
        -4, 4, 15, -1, 6, 17, 38, -5, //
        -43, -5, -5, -32, -20, -21, -53, -28, //
    ],
    end_game: [
        -11, -16, -6, -1, 0, -5, -16, -23, //
        -3, -8, 15, -7, 4, -9, 0, -13, //
        10, -4, 5, 7, 1, 14, 7, 8, //
        -1, 13, 18, 10, 16, 12, 7, 8, //
        -1, 3, 19, 23, 10, 13, 8, -9, //
        -9, 2, 18, 12, 18, 5, -7, -14, //
        -9, -13, -6, 8, 5, -5, -19, -26, //
        -17, -5, -12, 1, -4, -9, -1, -12, //
    ],
};
static ROOK: Table = Table {
    middle_game: [
        27, 40, 27, 52, 63, 2, 29, 37, //
        22, 30, 51, 66, 88, 72, 21, 41, //
        -11, 16, 25, 34, 16, 46, 62, 11, //
        -33, -19, 2, 27, 14, 29, -16, -33, //
        -48, -42, -24, -18, -2, -17, -3, -38, //
        -61, -43, -26, -32, -9, -7, -9, -52, //
        -58, -27, -34, -18, -5, 8, -17, -98, //
        -34, -20, -23, -1, 8, -10, -39, -41, //
    ],
    end_game: [
        14, 5, 16, 13, 7, 13, 11, -6, //
        10, 12, 4, 10, -1, 7, 11, -2, //
        13, 9, 13, 6, 6, -4, -5, -3, //
        14, 11, 19, 2, 0, 2, 3, 3, //
        10, 7, 14, 5, -7, -1, -8, -15, //
        -4, -4, -5, -2, -9, -12, -8, -17, //
        -10, -7, -3, -2, -13, -15, -18, -8, //
        -8, -6, 0, -15, -19, -18, 1, -17, //
    ],
};
static QUEEN: Table = Table {
    middle_game: [
        -34, -1, 32, 8, 65, 49, 46, 45, //
        -23, -45, -8, -2, -22, 63, 30, 54, //
        -13, -25, 8, 9, 32, 59, 51, 52, //
        -36, -30, -18, -25, -5, 19, -10, -4, //
        -8, -33, -8, -12, -8, -10, -4, -6, //
        -11, 2, -15, -6, -2, 1, 14, 2, //
        -43, -11, 6, 0, 6, 17, -10, -4, //
        -10, -24, 2, 12, -25, -30, -40, -64, //
    ],
    end_game: [
        -20, 16, 16, 20, 21, 10, 1, 11, //
        -28, 15, 27, 38, 58, 19, 24, -12, //
        -31, -4, 1, 46, 45, 29, 14, -1, //
        -8, 16, 19, 42, 57, 38, 56, 32, //
        -29, 22, 16, 44, 25, 31, 33, 18, //
        -27, -38, 7, -7, 4, 12, 2, -7, //
        -37, -38, -48, -28, -27, -38, -56, -50, //
        -51, -43, -33, -57, -21, -50, -35, -59, //
    ],
};
static KING: Table = Table {
    middle_game: [
        -58, 45, 37, 1, -49, -21, 21, 34, //
        53, 17, -5, 11, 10, 14, -26, -16, //
        7, 48, 21, -1, -5, 24, 43, -7, //
        -2, -5, 4, -14, -18, -12, 1, -24, //
        -39, 15, -14, -29, -38, -35, -22, -44, //
        1, 2, -12, -36, -35, -20, -1, -13, //
        21, 29, 7, -43, -25, 9, 25, 29, //
        4, 49, 38, -32, 12, -8, 45, 36, //
    ],
    end_game: [
        -91, -44, -24, -24, -16, 15, 1, -23, //
        -18, 17, 18, 17, 16, 41, 24, 8, //
        8, 19, 27, 17, 26, 43, 45, 12, //
        -15, 23, 27, 26, 29, 34, 27, 4, //
        -25, -7, 21, 27, 22, 20, 8, -18, //
        -26, -1, 4, 22, 27, 9, -2, -19, //
        -35, -17, 0, 19, 17, 5, -10, -37, //
        -60, -47, -21, -6, -33, -5, -39, -64, //
    ],
};

// ---- src/engine/evaluation/weights.rs ----
const KNIGHT_MOBILITY: [ScorePair; 9] = [
    ScorePair::new(-3, 0),
    ScorePair::new(4, 10),
    ScorePair::new(11, 25),
    ScorePair::new(18, 29),
    ScorePair::new(16, 39),
    ScorePair::new(19, 42),
    ScorePair::new(19, 47),
    ScorePair::new(22, 50),
    ScorePair::new(19, 49),
];
const BISHOP_MOBILITY: [ScorePair; 14] = [
    ScorePair::new(-14, -4),
    ScorePair::new(-2, 9),
    ScorePair::new(10, 20),
    ScorePair::new(11, 31),
    ScorePair::new(16, 39),
    ScorePair::new(25, 51),
    ScorePair::new(28, 55),
    ScorePair::new(34, 61),
    ScorePair::new(26, 67),
    ScorePair::new(27, 74),
    ScorePair::new(30, 76),
    ScorePair::new(33, 76),
    ScorePair::new(42, 97),
    ScorePair::new(45, 97),
];
const ROOK_MOBILITY: [ScorePair; 15] = [
    ScorePair::new(-17, -4),
    ScorePair::new(0, 12),
    ScorePair::new(6, 25),
    ScorePair::new(15, 35),
    ScorePair::new(22, 39),
    ScorePair::new(24, 48),
    ScorePair::new(20, 63),
    ScorePair::new(26, 65),
    ScorePair::new(26, 73),
    ScorePair::new(30, 78),
    ScorePair::new(32, 83),
    ScorePair::new(31, 85),
    ScorePair::new(37, 90),
    ScorePair::new(42, 96),
    ScorePair::new(42, 79),
];
const QUEEN_MOBILITY: [ScorePair; 28] = [
    ScorePair::new(0, 0),
    ScorePair::new(6, 7),
    ScorePair::new(9, 17),
    ScorePair::new(17, 24),
    ScorePair::new(18, 34),
    ScorePair::new(23, 43),
    ScorePair::new(28, 51),
    ScorePair::new(32, 63),
    ScorePair::new(36, 70),
    ScorePair::new(38, 79),
    ScorePair::new(39, 87),
    ScorePair::new(39, 97),
    ScorePair::new(46, 103),
    ScorePair::new(52, 115),
    ScorePair::new(57, 122),
    ScorePair::new(53, 128),
    ScorePair::new(60, 138),
    ScorePair::new(57, 145),
    ScorePair::new(60, 150),
    ScorePair::new(66, 157),
    ScorePair::new(69, 164),
    ScorePair::new(72, 171),
    ScorePair::new(77, 180),
    ScorePair::new(81, 187),
    ScorePair::new(84, 197),
    ScorePair::new(89, 207),
    ScorePair::new(93, 216),
    ScorePair::new(96, 224),
];
const ROOK_OPEN_FILE: ScorePair = ScorePair::new(36, -11);
const ROOK_SEMI_OPEN_FILE: ScorePair = ScorePair::new(5, 23);
const ROOK_ON_SEVENTH: ScorePair = ScorePair::new(-3, 9);
const KNIGHT_OUTPOST: ScorePair = ScorePair::new(22, 9);
const BISHOP_OUTPOST: ScorePair = ScorePair::new(5, 12);
const BACKWARD_PAWN: ScorePair = ScorePair::new(1, -5);
const THREAT_MINOR_BY_PAWN: ScorePair = ScorePair::new(49, 22);
const THREAT_HANGING: ScorePair = ScorePair::new(25, 22);
const THREAT_BY_LOWER_VALUE: ScorePair = ScorePair::new(37, 13);
const CONNECTED_PAWN_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(4, -3),
    ScorePair::new(13, 9),
    ScorePair::new(13, 13),
    ScorePair::new(22, 19),
    ScorePair::new(12, 14),
    ScorePair::new(0, -1),
];
const BLOCKED_PASSER_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-4, -10),
    ScorePair::new(-4, -3),
    ScorePair::new(-11, -6),
    ScorePair::new(-4, -16),
    ScorePair::new(-3, -20),
    ScorePair::new(-9, -30),
];
const PASSER_OWN_KING_DISTANCE: [ScorePair; 8] = [
    ScorePair::new(0, 7),
    ScorePair::new(1, 30),
    ScorePair::new(-3, 9),
    ScorePair::new(-2, -3),
    ScorePair::new(-10, -13),
    ScorePair::new(-6, -19),
    ScorePair::new(4, -14),
    ScorePair::new(-6, -12),
];
const PASSER_ENEMY_KING_DISTANCE: [ScorePair; 8] = [
    ScorePair::new(-15, -37),
    ScorePair::new(-5, -43),
    ScorePair::new(-3, -20),
    ScorePair::new(3, 4),
    ScorePair::new(-3, 22),
    ScorePair::new(6, 28),
    ScorePair::new(-1, 18),
    ScorePair::new(-1, 11),
];
const KING_DANGER_BY_BUCKET: [ScorePair; 16] = [
    ScorePair::new(0, -2),
    ScorePair::new(-5, -13),
    ScorePair::new(-12, -5),
    ScorePair::new(-3, -12),
    ScorePair::new(-3, -13),
    ScorePair::new(1, -13),
    ScorePair::new(6, 1),
    ScorePair::new(7, 3),
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
    ScorePair::new(23, -1),
    ScorePair::new(15, 15),
    ScorePair::new(25, 9),
    ScorePair::new(45, 15),
];
const SHELTER_KING_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(14, -6),
    ScorePair::new(2, 1),
    ScorePair::new(-2, -9),
    ScorePair::new(2, -2),
    ScorePair::new(0, -2),
    ScorePair::new(0, 0),
];
const SHELTER_ADJACENT_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(-3, -2),
    ScorePair::new(-12, 2),
    ScorePair::new(-1, -4),
    ScorePair::new(-1, -5),
    ScorePair::new(0, -3),
    ScorePair::new(0, 0),
];
const STORM_KING_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(4, 10),
    ScorePair::new(0, 2),
    ScorePair::new(3, 2),
    ScorePair::new(2, 5),
    ScorePair::new(5, 1),
    ScorePair::new(5, 1),
];
const STORM_ADJACENT_FILE_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(0, 0),
    ScorePair::new(-2, -1),
    ScorePair::new(-2, 3),
    ScorePair::new(-2, 3),
    ScorePair::new(5, 2),
    ScorePair::new(3, -2),
];
const BLOCKED_STORM_BY_DISTANCE: [ScorePair; 6] = [
    ScorePair::new(0, 0),
    ScorePair::new(-2, -4),
    ScorePair::new(1, -2),
    ScorePair::new(1, -1),
    ScorePair::new(2, -1),
    ScorePair::new(-1, -2),
];
const SPACE_AREA: ScorePair = ScorePair::new(-5, 2);
const SPACE_AREA_BY_PIECES: ScorePair = ScorePair::new(0, 0);
const CANDIDATE_PASSER_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(1, 0),
    ScorePair::new(2, 1),
    ScorePair::new(6, 9),
    ScorePair::new(2, 6),
    ScorePair::new(1, 1),
    ScorePair::new(0, 0),
];
const PAWN_ISLANDS: ScorePair = ScorePair::new(-3, 2);
const KING_PAWNLESS_FLANK: ScorePair = ScorePair::new(-3, -17);
const KNIGHT_PER_PAWN: ScorePair = ScorePair::new(1, 7);
const BISHOP_PER_PAWN: ScorePair = ScorePair::new(1, 4);
const ROOK_PER_PAWN: ScorePair = ScorePair::new(-7, 8);
const BISHOP_PAWNS_ON_COLOUR: ScorePair = ScorePair::new(-5, -3);
const UNSAFE_MOBILITY_BY_PIECE: [ScorePair; 4] = [
    ScorePair::new(-6, -2),
    ScorePair::new(1, -1),
    ScorePair::new(-4, 4),
    ScorePair::new(-2, 13),
];
const PASSER_SAFE_PATH_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(-1, 2),
    ScorePair::new(-2, 0),
    ScorePair::new(-2, 3),
    ScorePair::new(0, 8),
    ScorePair::new(2, 12),
    ScorePair::new(1, 3),
];
const PASSER_FREE_PATH_BY_RANK: [ScorePair; 6] = [
    ScorePair::new(0, 2),
    ScorePair::new(-2, 0),
    ScorePair::new(1, 3),
    ScorePair::new(1, 6),
    ScorePair::new(3, 6),
    ScorePair::new(2, 1),
];
const ROOK_BEHIND_PASSER: ScorePair = ScorePair::new(5, 4);
const THREAT_BY_PAWN_PUSH: ScorePair = ScorePair::new(10, 7);
const CASTLING_RIGHTS: ScorePair = ScorePair::new(18, 3);
const TROPISM_BY_PIECE_DISTANCE: [ScorePair; 16] = [
    ScorePair::new(-1, -3),
    ScorePair::new(-2, -4),
    ScorePair::new(2, 0),
    ScorePair::new(-7, 0),
    ScorePair::new(-1, -1),
    ScorePair::new(3, -2),
    ScorePair::new(-1, -1),
    ScorePair::new(-4, 4),
    ScorePair::new(0, 0),
    ScorePair::new(0, -3),
    ScorePair::new(-1, 1),
    ScorePair::new(-7, -1),
    ScorePair::new(0, 0),
    ScorePair::new(7, 4),
    ScorePair::new(3, 2),
    ScorePair::new(4, 4),
];
