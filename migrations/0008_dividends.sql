-- Equities and the index fund now pay dividends (income_yield, paid on
-- payday like rent). Price drift is lowered by the same amount so the
-- total expected return is unchanged. Only touches rows that haven't
-- been given a yield yet, so it is safe on any database.
UPDATE instruments SET income_yield = 0.005, drift = 0.115 WHERE symbol = 'AAPL' AND income_yield = 0;
UPDATE instruments SET income_yield = 0.040, drift = 0.030 WHERE symbol = 'NRGX' AND income_yield = 0;
UPDATE instruments SET income_yield = 0.025, drift = 0.035 WHERE symbol = 'GROV' AND income_yield = 0;
UPDATE instruments SET income_yield = 0.018, drift = 0.062 WHERE symbol = 'IDX1' AND income_yield = 0;
