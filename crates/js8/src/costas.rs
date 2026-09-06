pub(crate) const ORIGINAL_COSTAS: [[u8; 7]; 3] = [[4, 2, 5, 6, 1, 3, 0]; 3];

pub(crate) const MODIFIED_COSTAS: [[u8; 7]; 3] = [
    [0, 6, 2, 3, 5, 4, 1],
    [1, 5, 0, 2, 3, 6, 4],
    [2, 5, 0, 6, 4, 1, 3],
];

pub(crate) const COSTAS_SYMBOLS: usize = 7;
