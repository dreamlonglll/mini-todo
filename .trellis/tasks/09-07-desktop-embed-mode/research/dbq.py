import os, sqlite3, sys
db = os.path.expandvars(r'%LOCALAPPDATA%\mini-todo\data.db')
c = sqlite3.connect(db)
if len(sys.argv) > 1:
    for row in c.execute(sys.argv[1]).fetchall():
        print(row)
else:
    print('settings:', c.execute("select key,value from settings where key in ('is_fixed','is_desktop','window_x','window_y','window_width','window_height')").fetchall())
    print('screen_configs:', c.execute('select * from screen_configs').fetchall())
    print('migration:', c.execute('select max(version) from migrations').fetchall())
