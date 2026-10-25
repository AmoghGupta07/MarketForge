-- Buying or selling a home and repaying mortgage principal swap one kind
-- of asset for another, like trades do: they are not income or spending.
-- (Mortgage interest and upkeep are still expenses, category 'housing'.)
CREATE OR REPLACE FUNCTION is_transfer(cat TEXT) RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE AS $$
    SELECT cat IN ('trade', 'loan_draw', 'loan_principal', 'deposit', 'withdrawal', 'deal',
                   'business_invest', 'home', 'mortgage_principal')
$$;
